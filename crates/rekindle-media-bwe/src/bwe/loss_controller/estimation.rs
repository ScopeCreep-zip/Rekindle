//! The maximum-likelihood half of the loss controller (str0m
//! `src/bwe/loss_controller.rs`, `impl LossController`: candidate generation,
//! Newton's method, objective and bounds), moved into this file to stay under
//! the workspace's 600-line cap.

use std::time::Duration;

use super::super::time::TimeDelta;
use super::types::{AsValid, ChannelParameters};
use super::{LossController, LossControllerState};
use crate::util::{u64_as_usize, u64_to_f64};
use crate::Bitrate;

impl LossController {
    pub(super) fn get_candidates(&self) -> Vec<ChannelParameters> {
        let mut bandwidths = vec![];

        let current = self.current_estimate.loss_limited_bandwidth;

        for factor in &self.config.candidate_factor {
            bandwidths.push(factor * current.as_f64());
        }

        if self.delay_based_estimate.is_valid()
            && self.config.append_delay_based_estimate_candidate
            && self.delay_based_estimate > current
        {
            bandwidths.push(self.delay_based_estimate.as_f64());
        }

        let candidate_bandwidth_upper_bound = self.get_candidate_bandwidth_upper_bound().as_f64();

        if self.config.append_acknowledged_rate_candidate && self.acknowledged_bitrate.is_valid() {
            bandwidths.push(
                (self.acknowledged_bitrate * self.config.bandwidth_backoff_lower_bound_factor)
                    .as_f64(),
            );
        }

        if self.config.append_delay_based_estimate_candidate
            && self.delay_based_estimate.is_valid()
            && self.delay_based_estimate > current
        {
            bandwidths.push(
                (self.delay_based_estimate * self.config.bandwidth_backoff_lower_bound_factor)
                    .as_f64(),
            );
        }

        let mut candidates = Vec::with_capacity(bandwidths.len());

        for bandwidth in &mut bandwidths {
            let mut candidate = self.current_estimate;
            candidate.loss_limited_bandwidth = if self.config.trendline_integration_enabled {
                bandwidth.min(candidate_bandwidth_upper_bound).into()
            } else {
                bandwidth
                    .min(
                        self.current_estimate
                            .loss_limited_bandwidth
                            .as_f64()
                            .max(candidate_bandwidth_upper_bound),
                    )
                    .into()
            };
            candidate.inherent_loss = self.get_feasible_inherent_loss(&candidate);
            candidates.push(candidate);
        }

        candidates
    }

    pub(super) fn newtons_method_update(&self, channel_parameters: &mut ChannelParameters) {
        if self.num_observations == 0 {
            return;
        }

        for _ in 0..self.config.newton_iterations {
            let derivatives = self.get_derivatives(channel_parameters);
            channel_parameters.inherent_loss -=
                self.config.newton_step_size * (derivatives.0 / derivatives.1);
            channel_parameters.inherent_loss = self.get_feasible_inherent_loss(channel_parameters);
        }
    }

    pub(super) fn get_derivatives(&self, channel_prameters: &ChannelParameters) -> (f64, f64) {
        let mut derivatives: (f64, f64) = (0.0, 0.0);

        for observation in &self.observations {
            if !observation.is_initialized {
                continue;
            }

            let loss_probability = Self::get_loss_probability(
                channel_prameters.inherent_loss,
                channel_prameters.loss_limited_bandwidth,
                observation.sending_rate,
            );

            let index = (self.num_observations - 1) - observation.id;
            let temporal_weight = self.temporal_weights[u64_as_usize(index)];

            if self.config.use_byte_loss_ratio {
                derivatives.0 += temporal_weight
                    * ((observation.lost_size.as_kb() / loss_probability)
                        - ((observation.size - observation.lost_size).as_kb()
                            / (1.0 - loss_probability)));

                derivatives.1 -= temporal_weight
                    * ((observation.lost_size.as_kb() / f64::powi(loss_probability, 2))
                        + ((observation.size - observation.lost_size).as_kb()
                            / f64::powi(1.0 - loss_probability, 2)));
            } else {
                derivatives.0 += temporal_weight
                    * ((u64_to_f64(observation.num_lost_packets) / loss_probability)
                        - (u64_to_f64(observation.num_received_packets)
                            / (1.0 - loss_probability)));

                derivatives.1 -= temporal_weight
                    * ((u64_to_f64(observation.num_lost_packets) / f64::powi(loss_probability, 2))
                        + (u64_to_f64(observation.num_received_packets)
                            / f64::powi(1.0 - loss_probability, 2)));
            }
        }

        // Clamp second derivative to safe value if invalid due to floating-point edge cases
        // (infinity, denormals, extreme values from pathological TWCC feedback)
        if !derivatives.1.is_sign_negative() || derivatives.1 == 0.0 || derivatives.1.is_nan() {
            derivatives.1 = -1.0e-6;
            tracing::debug!(
                "Second derivative clamped to safe value due to invalid result: was {:?}",
                derivatives.1
            );
        }

        derivatives
    }

    pub(super) fn get_loss_probability(
        inherent_loss: f64,
        loss_limited_bandwidth: Bitrate,
        sending_rate: Bitrate,
    ) -> f64 {
        let inherent_loss = inherent_loss.clamp(0.0, 1.0);

        // maybe warn if sending rate or loss limited bandwidth are not finite

        let mut loss_probability = inherent_loss;
        if sending_rate.is_valid()
            && loss_limited_bandwidth.is_valid()
            && sending_rate > loss_limited_bandwidth
        {
            loss_probability += (1.0 - inherent_loss)
                * ((sending_rate - loss_limited_bandwidth).as_f64() / sending_rate.as_f64());
        }

        loss_probability.clamp(1.0e-6, 1.0 - 1.0e-6)
    }

    pub(super) fn get_objective(&self, candidate: &ChannelParameters) -> f64 {
        let mut objective = 0.0;
        let high_bandwidth_bias = self.get_high_bandwidth_bias(candidate.loss_limited_bandwidth);

        for observation in &self.observations {
            if !observation.is_initialized {
                continue;
            }

            let loss_probability = Self::get_loss_probability(
                candidate.inherent_loss,
                candidate.loss_limited_bandwidth,
                observation.sending_rate,
            );

            let index = (self.num_observations - 1) - observation.id;
            let temporal_weight = self.temporal_weights[u64_as_usize(index)];

            if self.config.use_byte_loss_ratio {
                objective += temporal_weight
                    * ((observation.lost_size.as_kb() / 1000.0) * f64::ln(loss_probability)
                        + ((observation.size - observation.lost_size).as_kb() / 1000.0)
                            * f64::ln(1.0 - loss_probability));
                objective +=
                    temporal_weight * high_bandwidth_bias * observation.size.as_kb() / 1000.0;
            } else {
                objective += temporal_weight
                    * (u64_to_f64(observation.num_lost_packets) * f64::ln(loss_probability)
                        + (u64_to_f64(observation.num_received_packets)
                            * f64::ln(1.0 - loss_probability)));

                objective +=
                    temporal_weight * high_bandwidth_bias * u64_to_f64(observation.num_packets);
            }
        }

        objective
    }

    pub(super) fn is_estimate_increasing_when_loss_limited(
        &self,
        candidate: ChannelParameters,
    ) -> bool {
        if !self.is_bandwidth_limited_due_to_loss() {
            return false;
        }

        let current = self.current_estimate.loss_limited_bandwidth;
        let candidate = candidate.loss_limited_bandwidth;

        if current < candidate {
            return true;
        }

        current == candidate && self.state == LossControllerState::Increasing
    }

    pub(super) fn is_bandwidth_limited_due_to_loss(&self) -> bool {
        self.state != LossControllerState::DelayBased
    }

    pub(super) fn get_candidate_bandwidth_upper_bound(&self) -> Bitrate {
        let mut upper_bound = self.max_bitrate;

        // When in ALR and we have a link capacity estimate from probes,
        // use it as the upper bound. This prevents estimating beyond proven capacity.
        if self.is_in_alr() {
            if let Some(capacity) = self.link_capacity_estimate {
                if capacity.is_valid() {
                    upper_bound = upper_bound.min(capacity);
                }
            }
        }

        if self.is_bandwidth_limited_due_to_loss()
            && self.bandwidth_limit_in_current_window.is_valid()
        {
            upper_bound = self.bandwidth_limit_in_current_window.min(upper_bound);
        }

        upper_bound = self.get_instant_upper_bound().min(upper_bound);
        if self.delay_based_estimate.is_valid() {
            upper_bound = upper_bound.min(self.delay_based_estimate);
        }

        if !self.acknowledged_bitrate.is_valid() {
            return upper_bound;
        }

        if self.config.rampup_acceleration_max_factor > Duration::ZERO
            && self.last_send_time_most_recent_observation.is_exact()
            && self.last_time_estimate_reduced.is_exact()
        {
            let delta = (self.last_send_time_most_recent_observation
                - self.last_time_estimate_reduced)
                .max(TimeDelta::ZERO);
            let time_since_bw_reduced = self
                .config
                .rampup_acceleration_maxout_time
                .as_secs_f64()
                .min(delta.as_secs_f64());

            let rampup_acceleration = self.config.rampup_acceleration_max_factor.as_secs_f64()
                * time_since_bw_reduced
                / self.config.rampup_acceleration_maxout_time.as_secs_f64();

            upper_bound = upper_bound + (self.acknowledged_bitrate * rampup_acceleration);
        }

        upper_bound
    }

    pub(super) fn set_state(&mut self, state: LossControllerState) {
        if state != self.state {
            tracing::debug!(
                "Changing loss controller state: {:?} -> {:?}",
                self.state,
                state
            );
        }
        self.state = state;
    }

    pub(super) fn get_high_bandwidth_bias(&self, bandwidth: Bitrate) -> f64 {
        if !bandwidth.is_valid() {
            return 0.0;
        }

        let average_reported_loss_ratio = self.average_reported_loss_ratio();

        self.adjust_bias_factor(
            average_reported_loss_ratio,
            self.config.higher_bandwidth_bias_factor,
        ) * bandwidth.as_f64()
            + self.adjust_bias_factor(
                average_reported_loss_ratio,
                self.config.higher_log_bandwidth_bias_factor,
            ) * f64::ln(1.0 + bandwidth.as_f64())
    }

    pub(super) fn adjust_bias_factor(&self, loss_rate: f64, bias_factor: f64) -> f64 {
        let diff = self.config.threshold_of_high_bandwidth_preference - loss_rate;
        bias_factor * (diff / self.config.bandwidth_preference_smoothing_factor + diff.abs())
    }

    pub(super) fn calculate_instant_upper_bound(&self) -> Bitrate {
        // this requires someone to set the max bitrate from outside
        let mut instant_limit = self.max_bitrate;

        let average_reported_loss_ratio = self.average_reported_loss_ratio();

        if average_reported_loss_ratio > self.config.instant_upper_bound_loss_offset {
            instant_limit = self.config.instant_upper_bound_bandwidth_balance
                / (average_reported_loss_ratio - self.config.instant_upper_bound_loss_offset);

            if average_reported_loss_ratio > self.config.high_loss_rate_threshold {
                let limit = self.config.bandwidth_cap_at_high_loss_rate
                    - self.config.slope_of_bwe_high_loss_function * average_reported_loss_ratio;

                instant_limit = limit.max(self.min_bitrate);
            }
        }

        instant_limit
    }

    pub(super) fn get_instant_upper_bound(&self) -> Bitrate {
        self.cached_instant_upper_bound
            .as_valid()
            .unwrap_or(self.max_bitrate)
    }

    pub(super) fn average_reported_loss_ratio(&self) -> f64 {
        let mut total = 0_f64;
        let mut lost = 0_f64;

        for observation in &self.observations {
            if !observation.is_initialized {
                continue;
            }

            let index = (self.num_observations - 1) - observation.id;

            let instant_temporal_weight =
                self.instant_upper_bound_temporal_weights[u64_as_usize(index)];

            if self.config.use_byte_loss_ratio {
                total += instant_temporal_weight * observation.size.as_bytes_f64();
                lost += instant_temporal_weight * observation.lost_size.as_bytes_f64();
            } else {
                total += instant_temporal_weight * u64_to_f64(observation.num_packets);
                lost += instant_temporal_weight * u64_to_f64(observation.num_lost_packets);
            }
        }

        if total == 0_f64 {
            return 0.0;
        }

        lost / total
    }

    pub(super) fn get_feasible_inherent_loss(&self, channel_parameters: &ChannelParameters) -> f64 {
        channel_parameters
            .inherent_loss
            .max(self.config.inherent_loss_lower_bound)
            .min(
                self.get_inherent_loss_upper_bound(Some(channel_parameters.loss_limited_bandwidth)),
            )
    }

    pub(super) fn get_inherent_loss_upper_bound(&self, bandwidth: Option<Bitrate>) -> f64 {
        let Some(bandwidth) = bandwidth else {
            return 1.0;
        };

        if bandwidth == Bitrate::ZERO {
            return 1.0;
        }

        let inherent_loss_upper_bound = self.config.inherent_loss_upper_bound_offset
            + self
                .config
                .inherent_loss_upper_bound_bandwidth_balance
                .as_f64()
                / bandwidth.as_f64();

        inherent_loss_upper_bound.min(1.0)
    }
}
