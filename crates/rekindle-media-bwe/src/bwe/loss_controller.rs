use std::time::{Duration, Instant};

use super::macros::log_inherent_loss;
use super::macros::log_loss_based_bitrate_estimate;
use super::macros::log_loss_bw_limit_in_window;
use crate::util::{u64_as_usize, usize_as_i32};
use crate::{Bitrate, DataSize};

use super::time::BweTimestamp;

mod estimation;
mod types;

use types::{
    ChannelParameters, Config, HoldInfo, Observation, PacketResultsSummary, PartialObservation,
};

/// Loss controller based on libWebRTC's `LossBasedBweV2`.
///
/// ## Overview
///
/// The estimator attempts to estimate the inherent loss of the link using Maximum Likelihood
/// Estimation of an assumed Bernoulli distribution. This allows it to distinguish congestion
/// induced loss from this inherent loss.
///
/// The controller integrates with ALR (Application Limited Region) detection and link capacity
/// tracking. When in ALR, it uses proven link capacity from successful ALR probes as an upper
/// bound on estimates, preventing overestimation in application-limited scenarios. When
/// transitioning into or out of ALR, the controller resets its observation window to avoid
/// mixing traffic patterns from different network utilization regimes.
///
/// The estimate is bounded by the output of the delay-based estimator, meaning this controller
/// can only reduce estimates (acting as a safety cap), not increase them.
///
///
/// Ref:
/// * https://webrtc.googlesource.com/src/+/refs/heads/main/modules/congestion_controller/goog_cc/loss_based_bwe_v2.cc
/// * https://webrtc.googlesource.com/src/+/refs/heads/main/modules/congestion_controller/goog_cc/loss_based_bwe_v2.h
pub struct LossController {
    /// Configuration for the controller.
    config: Config,

    /// The current state of the controller.
    state: LossControllerState,

    /// Staging ground for observations while they are being constructed.
    partial_observation: PartialObservation,

    /// The last packet sent in the most recent observation.
    last_send_time_most_recent_observation: BweTimestamp,

    // Observation window
    /// Forever growing counter of observations. Observation::id derives from this.
    num_observations: u64,
    /// Window of observations.
    observations: Box<[Observation]>,
    /// Temporal weights, used to weight observations by recency. Same size as `observations`.
    temporal_weights: Box<[f64]>,
    /// Upper bound temporal weights, used to weight observations by recency. Same size as `observations`.
    instant_upper_bound_temporal_weights: Box<[f64]>,

    /// Precomputed instantaneous upper bound on bandwidth estimate.
    cached_instant_upper_bound: Option<Bitrate>,
    /// Last time we reduced the estimate.
    last_time_estimate_reduced: BweTimestamp,

    /// When we started recovering after being loss limited last time.
    /// While in this window the bandwidth estimate is bounded by `bandwidth_limit_in_current_window`.
    recovering_after_loss_timestamp: BweTimestamp,
    /// Upper bound on estimate while in recovery window.
    bandwidth_limit_in_current_window: Bitrate,

    /// The current estimate
    current_estimate: ChannelParameters,

    /// The min bitrate we will emit as an estimate.
    min_bitrate: Bitrate,
    /// The max bitrate we will emit as an estimate.
    max_bitrate: Bitrate,

    /// The most recent acknowledged bitrate derived from TWCC.
    acknowledged_bitrate: Bitrate,

    /// The most recent estimated bitrate from the delay based estimator.
    delay_based_estimate: Bitrate,

    /// HOLD mechanism state - prevents immediate ramp-up after loss
    last_hold_info: HoldInfo,

    /// ALR start time (None if not in ALR)
    alr_start_time: Option<Instant>,

    /// Link capacity estimate from probes during ALR
    link_capacity_estimate: Option<Bitrate>,

    /// Previous ALR state to detect transitions
    was_in_alr: bool,
}

/// State of the Loss Controller
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum LossControllerState {
    /// LossController is in increasing state
    Increasing,
    /// LossController is in decreasing state
    Decreasing,
    /// LossController is in relaying the estimate of the delay controller
    DelayBased,
}

pub trait PacketResult {
    /// When the packet was sent
    fn local_send_time(&self) -> Instant;
    /// Size of the packet payload
    fn size(&self) -> DataSize;

    /// Whether this packet was lost or not.
    fn lost(&self) -> bool;
}

impl LossController {
    pub fn new() -> LossController {
        let config = Config::default();

        let mut controller = LossController {
            state: LossControllerState::DelayBased,
            partial_observation: PartialObservation::new(),
            last_send_time_most_recent_observation: BweTimestamp::DistantFuture,
            observations: vec![Observation::DUMMY; config.observation_window_size]
                .into_boxed_slice(),
            num_observations: 0,
            temporal_weights: vec![0_f64; config.observation_window_size].into_boxed_slice(),
            instant_upper_bound_temporal_weights: vec![0_f64; config.observation_window_size]
                .into_boxed_slice(),
            cached_instant_upper_bound: None,
            last_time_estimate_reduced: BweTimestamp::DistantPast,
            recovering_after_loss_timestamp: BweTimestamp::DistantPast,
            bandwidth_limit_in_current_window: Bitrate::MAX,

            current_estimate: ChannelParameters::new(config.initial_inherent_loss_estimate),

            min_bitrate: Bitrate::kbps(1),
            max_bitrate: Bitrate::INFINITY,

            // review usage from here on after
            acknowledged_bitrate: Bitrate::INFINITY,
            delay_based_estimate: Bitrate::INFINITY,

            last_hold_info: HoldInfo::default(),

            alr_start_time: None,
            link_capacity_estimate: None,
            was_in_alr: false,

            config,
        };

        // Initialize weights
        {
            let this = &mut controller;
            for i in 0..this.config.observation_window_size {
                let val = f64::powi(this.config.temporal_weight_factor, usize_as_i32(i));
                this.temporal_weights[i] = val;
                let val = f64::powi(
                    this.config.instant_upper_bound_temporal_weight_factor,
                    usize_as_i32(i),
                );
                this.instant_upper_bound_temporal_weights[i] = val;
            }
        };

        controller
    }

    /// Override the current bandwidth estimate.
    pub fn set_bandwidth_estimate(&mut self, bandwidth_estimate: Bitrate) {
        self.current_estimate.loss_limited_bandwidth = bandwidth_estimate;
    }

    /// Update the acknowledged bitrate based on TWCC feedback.
    pub fn set_acknowledged_bitrate(&mut self, acknowledged_bitrate: Bitrate) {
        self.acknowledged_bitrate = acknowledged_bitrate;
    }

    /// Set ALR start time from the ALR detector.
    pub fn set_alr_start_time(&mut self, alr_start: Option<Instant>) {
        let was_in_alr = self.was_in_alr;
        let is_in_alr = alr_start.is_some();

        // Detect ALR state transition
        if was_in_alr != is_in_alr {
            // Reset observations on ALR transition to avoid mixing
            // ALR and non-ALR traffic in the same observation window
            self.reset_observations();
            tracing::trace!(
                "LossController: ALR state changed (was: {}, now: {}), observations reset",
                was_in_alr,
                is_in_alr
            );
        }

        self.alr_start_time = alr_start;
        self.was_in_alr = is_in_alr;
    }

    /// Set link capacity estimate from successful ALR probes.
    pub fn set_link_capacity_estimate(&mut self, capacity: Option<Bitrate>) {
        self.link_capacity_estimate = capacity;
    }

    /// Check if currently in ALR state
    fn is_in_alr(&self) -> bool {
        self.alr_start_time.is_some()
    }

    /// Reset all observations.
    ///
    /// Called when ALR state transitions to avoid mixing traffic patterns
    /// from different network utilization regimes.
    fn reset_observations(&mut self) {
        self.partial_observation = PartialObservation::new();
        self.last_send_time_most_recent_observation = BweTimestamp::DistantFuture;

        // Clear observation window
        for observation in &mut self.observations {
            *observation = Observation::DUMMY;
        }

        // Reset cached values that depend on observations
        self.cached_instant_upper_bound = None;
    }

    /// Update the estimate using TWCC feedback from the network.
    /// After this [`loss_based_result`] returns the latest estimate.
    pub fn update_bandwidth_estimate(
        &mut self,
        packet_results: &[impl PacketResult],
        delay_based_estimated: Bitrate,
    ) {
        self.delay_based_estimate = delay_based_estimated;

        if packet_results.is_empty() {
            tracing::debug!("packet results is empty");
            return;
        }

        if !self.maybe_add_observation(packet_results) {
            return;
        }

        if !self.current_estimate.loss_limited_bandwidth.is_valid() {
            tracing::warn!("estimator must be initialized before use");
            return;
        }

        let mut best_candidate = self.current_estimate;
        let mut objective_max = f64::MIN;

        for candidate in &mut self.get_candidates() {
            self.newtons_method_update(candidate);

            let candidate_objective = self.get_objective(candidate);
            if candidate_objective > objective_max {
                objective_max = candidate_objective;
                best_candidate = *candidate;
            }
        }

        if best_candidate.loss_limited_bandwidth < self.current_estimate.loss_limited_bandwidth {
            self.last_time_estimate_reduced = self.last_send_time_most_recent_observation;
        }

        // do not increase the estimate if the average loss is greater than current inherent loss
        if self.average_reported_loss_ratio() > best_candidate.inherent_loss
            && self
                .config
                .not_increase_if_inherent_loss_less_than_average_loss
            && self.current_estimate.loss_limited_bandwidth < best_candidate.loss_limited_bandwidth
        {
            best_candidate.loss_limited_bandwidth = self.current_estimate.loss_limited_bandwidth;
        }

        if self.is_bandwidth_limited_due_to_loss() {
            // Bound the estimate increase if:
            // 1. The estimate has been increased for less than
            // `delayed_increase_window` ago, and
            // 2. The best candidate is greater than bandwidth_limit_in_current_window.

            if self.recovering_after_loss_timestamp.is_exact()
                && self.recovering_after_loss_timestamp + self.config.delayed_increase_window
                    > self.last_send_time_most_recent_observation
                && best_candidate.loss_limited_bandwidth > self.bandwidth_limit_in_current_window
            {
                best_candidate.loss_limited_bandwidth = self.bandwidth_limit_in_current_window;
            }

            let increase_when_loss_limited =
                self.is_estimate_increasing_when_loss_limited(best_candidate);

            if increase_when_loss_limited && self.acknowledged_bitrate.is_valid() {
                // Choose rampup factor based on whether we're in HOLD region (WebRTC lines 270-281)
                let rampup_factor = if self.last_hold_info.rate.is_valid()
                    && self.acknowledged_bitrate
                        < self.last_hold_info.rate * self.config.bandwidth_rampup_hold_threshold
                {
                    self.config.bandwidth_rampup_upper_bound_factor_in_hold // 1.2
                } else {
                    self.config.bandwidth_rampup_upper_bound_factor // 1.5
                };

                best_candidate.loss_limited_bandwidth =
                    self.current_estimate.loss_limited_bandwidth.max(
                        best_candidate
                            .loss_limited_bandwidth
                            .min(self.acknowledged_bitrate * rampup_factor),
                    );

                // WebRTC lines 282-290: Ensure at least 1 bps increase when transitioning from Decreasing
                if self.state == LossControllerState::Decreasing
                    && best_candidate.loss_limited_bandwidth
                        == self.current_estimate.loss_limited_bandwidth
                {
                    best_candidate.loss_limited_bandwidth =
                        self.current_estimate.loss_limited_bandwidth + Bitrate::bps(1);
                }
            }
        }

        let loss_limited_bandwidth = best_candidate.loss_limited_bandwidth;

        // HOLD check (WebRTC lines 321-334): If in Decreasing state and HOLD timer active, cap at HOLD rate
        if self.state == LossControllerState::Decreasing
            && self.last_hold_info.timestamp > self.last_send_time_most_recent_observation
            && loss_limited_bandwidth < self.delay_based_estimate
        {
            // During HOLD period, cap estimate at HOLD rate
            self.current_estimate = best_candidate;
            self.current_estimate.loss_limited_bandwidth =
                loss_limited_bandwidth.min(self.last_hold_info.rate);
            log_inherent_loss!(self.current_estimate.inherent_loss);
            log_loss_based_bitrate_estimate!(self.current_estimate.loss_limited_bandwidth.as_f64());
            return;
        }

        // State transitions with HOLD mechanism (WebRTC lines 336-378)
        let new_state = if self.is_estimate_increasing_when_loss_limited(best_candidate)
            && loss_limited_bandwidth < delay_based_estimated
            && loss_limited_bandwidth < self.max_bitrate
        {
            LossControllerState::Increasing
        } else if loss_limited_bandwidth < self.delay_based_estimate
            && loss_limited_bandwidth < self.max_bitrate
        {
            // Entering Decreasing state - set HOLD info
            if self.state != LossControllerState::Decreasing
                && self.config.hold_duration_factor > 0.0
            {
                const MAX_HOLD_DURATION: Duration = Duration::from_secs(60);
                self.last_hold_info = HoldInfo {
                    timestamp: self.last_send_time_most_recent_observation
                        + self.last_hold_info.duration,
                    duration: MAX_HOLD_DURATION.min(Duration::from_secs_f64(
                        self.last_hold_info.duration.as_secs_f64()
                            * self.config.hold_duration_factor,
                    )),
                    rate: loss_limited_bandwidth,
                };
            }
            LossControllerState::Decreasing
        } else {
            // Reset HOLD info when returning to DelayBased
            self.last_hold_info = HoldInfo {
                timestamp: BweTimestamp::DistantPast,
                duration: Duration::from_millis(300),
                rate: Bitrate::INFINITY,
            };
            LossControllerState::DelayBased
        };
        self.set_state(new_state);

        self.current_estimate = best_candidate;
        log_inherent_loss!(self.current_estimate.inherent_loss);
        log_loss_based_bitrate_estimate!(self.current_estimate.loss_limited_bandwidth.as_f64());

        const CONGESTION_CONTROLLER_MIN_BITRATE: Bitrate = Bitrate::kbps(5);
        const CONF_MAX_INCREASE_FACTOR: f64 = 1.3;

        if self.is_bandwidth_limited_due_to_loss()
            && (!self.recovering_after_loss_timestamp.is_exact()
                || self.recovering_after_loss_timestamp + self.config.delayed_increase_window
                    < self.last_send_time_most_recent_observation)
        {
            self.bandwidth_limit_in_current_window = CONGESTION_CONTROLLER_MIN_BITRATE
                .max(loss_limited_bandwidth * CONF_MAX_INCREASE_FACTOR);

            self.recovering_after_loss_timestamp = self.last_send_time_most_recent_observation;
            log_loss_bw_limit_in_window!(self.bandwidth_limit_in_current_window.as_f64());
        }
    }

    // TODO: Determine if we want to integrate these two with the rest of the system.
    #[cfg(test)]
    pub fn set_max_bitrate(&mut self, max_bitrate: Bitrate) {
        self.max_bitrate = max_bitrate;
    }

    #[cfg(test)]
    pub fn set_min_bitrate(&mut self, min_bitrate: Bitrate) {
        self.min_bitrate = min_bitrate;
    }

    pub fn loss_based_result(&self) -> LossBasedBweResult {
        let mut result = LossBasedBweResult {
            bandwidth_estimate: self.current_estimate.loss_limited_bandwidth.as_valid(),
            state: self.state,
        };

        if self.num_observations == 0 {
            return result;
        }

        let Some(loss_limited_bandwidth) = self.current_estimate.loss_limited_bandwidth.as_valid()
        else {
            return result;
        };
        let instant_upper_bound = self.get_instant_upper_bound();

        if self.delay_based_estimate.is_valid() {
            result.bandwidth_estimate = Some(
                loss_limited_bandwidth
                    .min(self.delay_based_estimate)
                    .min(instant_upper_bound),
            );
        } else {
            result.bandwidth_estimate = Some(loss_limited_bandwidth.min(instant_upper_bound));
        }

        result
    }

    fn maybe_add_observation(&mut self, packet_results: &[impl PacketResult]) -> bool {
        let Some(summary) = PacketResultsSummary::from(packet_results) else {
            return false;
        };

        let last_send_time = BweTimestamp::from(summary.last_send_time);

        self.partial_observation.update(&summary);

        if !self.last_send_time_most_recent_observation.is_exact() {
            self.last_send_time_most_recent_observation = last_send_time;
        }

        let observation_duration = last_send_time - self.last_send_time_most_recent_observation;

        if observation_duration <= Duration::ZERO {
            return false;
        }

        // decide if we can accept the partial observation as complete
        if observation_duration <= self.config.observation_duration_lower_bound {
            return false;
        }

        self.last_send_time_most_recent_observation = last_send_time;

        let observation = {
            let id = self.num_observations;
            self.num_observations += 1;

            Observation {
                num_packets: self.partial_observation.num_packets,
                size: self.partial_observation.size,
                num_lost_packets: self.partial_observation.num_lost_packets,
                lost_size: self.partial_observation.lost_size,
                num_received_packets: self.partial_observation.num_packets
                    - self.partial_observation.num_lost_packets,
                sending_rate: self.partial_observation.size / observation_duration,
                id,
                is_initialized: true,
            }
        };

        // save our complete observation
        self.observations[u64_as_usize(observation.id) % self.config.observation_window_size] =
            observation;

        // renew the partial observation
        self.partial_observation = PartialObservation::new();

        // calculate upper bound
        self.cached_instant_upper_bound = Some(self.calculate_instant_upper_bound());

        true
    }
}

#[derive(Debug)]
pub struct LossBasedBweResult {
    pub bandwidth_estimate: Option<Bitrate>,
    pub state: LossControllerState,
}

#[cfg(test)]
mod test;
