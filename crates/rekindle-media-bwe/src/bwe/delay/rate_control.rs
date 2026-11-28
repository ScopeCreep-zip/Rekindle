use std::fmt;
use std::time::{Duration, Instant};

use super::super::macros::log_rate_control_applied_change;
use super::super::macros::log_rate_control_observed_bitrate;
use super::super::macros::log_rate_control_state;
use crate::util::{f64_to_u64, MovingAverage};
use crate::Bitrate;

use super::super::BandwidthUsage;

// Recommended values from https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02#section-5
/// Smoothing factor applied to moving stats for observed bitrates when we are in the decreasing
/// state.
const OBSERVED_BIT_RATE_SMOOTHING_FACTOR: f64 = 0.95;
/// The ratio of current estimated bandwidth to use when decreasing the rate.
const BETA: f64 = 0.85;
/// The coefficient used for multiplicative rate increase.
const MULTIPLICATIVE_INCREASE_COEF: f64 = 1.08;
/// The maximal ratio of the observed bitrate that we allow estimating in a single increase.
const MAX_ESTIMATE_RATIO: f64 = 1.5;
/// Default backoff time added to RTT for response time calculation (kDefaultBackoffTimeInMs in WebRTC).
const DEFAULT_BACKOFF_TIME: Duration = Duration::from_millis(100);
/// RTT assumed before one is measured (libwebrtc `kDefaultRtt`).
const DEFAULT_RTT: Duration = Duration::from_millis(200);
/// Packet size the near-max increase assumes (libwebrtc `kPacketSize`).
const PACKET_SIZE_BITS: f64 = 1200.0 * 8.0;
/// Floor of the near-max increase (libwebrtc
/// `kMinIncreaseRateBpsPerSecond`).
const MIN_NEAR_MAX_INCREASE_BPS_PER_S: f64 = 4_000.0;
/// Number of standard deviations below mean to reset observed bitrate average.
const OBSERVED_BITRATE_RESET_THRESHOLD_STD: f64 = 3.0;

/// A type used to estimates a suitable send bitrate.
///
/// Inputs to the rate controller are:
/// * The observed received bitrate(via TWCC feddback).
/// * RTT.
/// * Congestion estimates from the delay controller.
pub struct RateControl {
    state: State,

    estimated_bitrate: Bitrate,
    min_bitrate: Bitrate,
    max_bitrate: Bitrate,

    /// The last observed bitrate calculated based on TWCC data.
    last_observed_bitrate: Option<Bitrate>,
    /// The averaged observed bitrate when we have been in the decrease state.
    averaged_observed_bitrate: MovingAverage,
    /// The last time we updated the estimated bitrate.
    last_estimate_update: Option<Instant>,
    // Last RTT estimate in micro-seconds
    last_rtt: Option<Duration>,
}

impl RateControl {
    pub fn new(start_bitrate: Bitrate, min_bitrate: Bitrate, max_bitrate: Bitrate) -> Self {
        log_rate_control_state!(State::Increase as i8);

        Self {
            state: State::Increase,

            estimated_bitrate: start_bitrate,
            min_bitrate,
            max_bitrate,

            last_observed_bitrate: None,
            averaged_observed_bitrate: MovingAverage::new(OBSERVED_BIT_RATE_SMOOTHING_FACTOR),
            last_estimate_update: None,
            last_rtt: None,
        }
    }

    /// Update with input from the delay controller.
    pub fn update(
        &mut self,
        signal: Signal,
        observed_bitrate: Bitrate,
        rtt: Option<Duration>,
        now: Instant,
    ) {
        self.last_observed_bitrate = Some(observed_bitrate);
        if let Some(rtt) = rtt {
            self.last_rtt = Some(rtt);
        }

        self.state = self.state.transition(signal);
        log_rate_control_observed_bitrate!(
            observed_bitrate.as_f64(),
            self.averaged_observed_bitrate
                .get()
                .map(|avg| avg.to_string())
                .unwrap_or_default()
        );

        match self.state {
            State::Increase => {
                self.increase(observed_bitrate, now);
            }
            State::Decrease => {
                // Maintain observed bitrate statistics while we are in the decrease state.
                //
                // This must NOT be gated by time_to_reduce_further. That function is
                // intended to gate *applying another reduction*, not collecting throughput stats.
                self.update_observed_bitrate(observed_bitrate);

                // Only apply decrease if enough time has passed since last bitrate change
                // or if throughput is critically low (< 50% of estimate)
                if self.time_to_reduce_further(now, observed_bitrate) {
                    self.decrease(observed_bitrate, now);
                }
            }
            State::Hold => {
                // Do nothing
            }
        }
    }

    fn update_observed_bitrate(&mut self, observed_bitrate: Bitrate) {
        if self
            .averaged_observed_bitrate
            .lower_range(OBSERVED_BITRATE_RESET_THRESHOLD_STD)
            .is_some_and(|lower| observed_bitrate.as_f64() < lower)
        {
            self.averaged_observed_bitrate.reset();
        }
        self.averaged_observed_bitrate
            .update(observed_bitrate.as_f64());
    }

    /// Check if it's time to reduce the bitrate further.
    ///
    /// This implements WebRTC's TimeToReduceFurther logic which prevents
    /// rapid successive decreases, especially after probe results.
    ///
    /// Returns true if:
    /// 1. Enough time (1 RTT, clamped to 10-200ms) has passed since last change, OR
    /// 2. Throughput is critically low (< 50% of current estimate)
    fn time_to_reduce_further(&self, now: Instant, observed_bitrate: Bitrate) -> bool {
        let Some(last_change) = self.last_estimate_update else {
            return true; // No previous change, allow decrease
        };

        // WebRTC uses: clamp(rtt, 10ms, 200ms)
        let rtt = self.last_rtt.unwrap_or(DEFAULT_BACKOFF_TIME);
        let reduction_interval = rtt.clamp(Duration::from_millis(10), Duration::from_millis(200));

        let time_since_change = now.saturating_duration_since(last_change);

        if time_since_change >= reduction_interval {
            return true;
        }

        // If throughput is critically low (< 50% of estimate), allow immediate decrease
        let threshold = self.estimated_bitrate * 0.5;
        if observed_bitrate < threshold {
            return true;
        }

        false
    }

    /// The current estimated bitrate.
    pub fn estimated_bitrate(&self) -> Bitrate {
        self.estimated_bitrate
    }

    /// Set a probe result indicating discovered capacity.
    ///
    /// When a probe succeeds, it means the network can handle at least this bitrate.
    /// We use this to quickly increase our estimate without waiting for gradual ramp-up.
    ///
    /// Apply a probe result directly to the estimate.
    ///
    /// This matches WebRTC's behavior where probe results are accepted unconditionally
    /// (subject only to min/max clamping), regardless of whether they're higher or lower
    /// than the current estimate. The timestamp is updated to prevent the next regular
    /// update from immediately overriding the probe result.
    ///
    /// WebRTC does NOT change the rate control state when applying a probe - the state
    /// remains unchanged to avoid triggering unintended AIMD behavior.
    pub fn set_probe_result(&mut self, probe_bitrate: Bitrate, now: Instant) {
        // WebRTC calls SetEstimate() directly without filtering by current estimate
        // or changing the rate control state. Accept the probe result unconditionally
        // (update_estimate handles clamping to min/max bounds).
        self.update_estimate(probe_bitrate, now);

        // Do NOT change self.state - keep current state to match WebRTC behavior.
        // Changing state here could trigger unintended AIMD decrease/increase logic.
    }

    fn increase(&mut self, observed_bitrate: Bitrate, now: Instant) {
        // WebRTC limits increases to 1.5x observed throughput to avoid unlimited growth
        // when we're already above what we're actually sending
        // See: aimd_rate_control.cc line 251-252
        let increase_limit = observed_bitrate * 1.5 + Bitrate::kbps(10);

        if self.estimated_bitrate >= increase_limit {
            // WebRTC updates time_last_bitrate_change_ even when skipping increase
            // (see aimd_rate_control.cc line 281, which is outside the increase check)
            // This prevents stale timestamps from allowing premature decreases
            self.last_estimate_update = Some(now);
            return;
        }

        // Initialize timestamp if this is the first increase call, otherwise use existing value.
        // Note: In practice, last_estimate_update is always Some here because either:
        // 1) We returned early above and set it, or
        // 2) A previous call to decrease() or increase() already set it
        let last_estimate_update = self.last_estimate_update.unwrap_or(now);
        if self.last_estimate_update.is_none() {
            self.last_estimate_update = Some(now);
        }

        if self
            .averaged_observed_bitrate
            .upper_range(3.0)
            .is_some_and(|upper| observed_bitrate.as_f64() > upper)
        {
            self.averaged_observed_bitrate.reset();
        }

        let since_last_update = now - last_estimate_update;
        assert!(since_last_update >= Duration::ZERO);
        let near_convergence = self.is_near_convergence();

        let mut new_estimate = if near_convergence {
            // Additive increase
            log_rate_control_applied_change!("increase_additive");
            // libwebrtc `AdditiveRateIncrease`: the near-max rate times the
            // time since the last change.
            self.estimated_bitrate.as_f64()
                + self.near_max_increase_bps_per_s() * since_last_update.as_secs_f64()
        } else {
            // Multiplicative increase
            log_rate_control_applied_change!("increase_multiplicative");
            let eta = MULTIPLICATIVE_INCREASE_COEF.powf(since_last_update.as_secs_f64().min(1.0));
            let increase = ((eta - 1.0) * self.estimated_bitrate.as_f64()).max(1_000.0);

            self.estimated_bitrate.as_f64() + increase
        };

        // Cap at the increase limit (and observed bitrate ratio)
        let max = observed_bitrate.as_f64() * MAX_ESTIMATE_RATIO;
        new_estimate = max.min(new_estimate).min(increase_limit.as_f64());

        self.update_estimate(new_estimate.into(), now);
    }

    fn decrease(&mut self, observed_bitrate: Bitrate, now: Instant) {
        log_rate_control_applied_change!("decrease");
        let mut new_estimate = observed_bitrate * BETA;

        if self.estimated_bitrate < new_estimate {
            // Avoid increasing the bitrate on overuse
            new_estimate = self.estimated_bitrate;
        }

        if let Some(observed_average) = self.averaged_observed_bitrate.get() {
            log_rate_control_observed_bitrate!(
                observed_bitrate.as_u64(),
                f64_to_u64(observed_average.round())
            );
        }
        // According to https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02#section-6 we
        // should wait until this happens as consequence of the delay control, but libWebRTC does
        // it immediately.
        self.state = State::Hold;
        log_rate_control_state!(self.state as i8);
        self.update_estimate(new_estimate, now);
    }

    fn is_near_convergence(&self) -> bool {
        // Not near convergence until we have valid statistics
        if !self.averaged_observed_bitrate.valid() {
            return false;
        }
        let Some(last_observed_bitrate) = self.last_observed_bitrate else {
            return false;
        };

        // Near convergence if the observed bandwidth is within 3 standard deviations of
        // the moving average when we have been in the decrease state.
        self.averaged_observed_bitrate
            .within_std(last_observed_bitrate.as_f64(), 3.0)
    }

    fn update_estimate(&mut self, bitrate: Bitrate, now: Instant) {
        self.estimated_bitrate = bitrate.clamp(self.min_bitrate, self.max_bitrate);
        self.last_estimate_update = Some(now);
    }

    /// libwebrtc `GetNearMaxIncreaseRateBpsPerSecond`
    /// (`aimd_rate_control.cc`): one average packet per response time,
    /// assuming 30 fps video in 1,200-byte packets, at least 4 kbps/s.
    /// (str0m divided bits by 1200/8, a bytes-to-bits slip that made the
    /// packet ~150 bits and the increase its 1 kbps floor.)
    fn near_max_increase_bps_per_s(&self) -> f64 {
        let frame_bits = self.estimated_bitrate.as_f64() / 30.0;
        let packets_per_frame = (frame_bits / PACKET_SIZE_BITS).ceil().max(1.0);
        let avg_packet_bits = frame_bits / packets_per_frame;
        let response_time = (self.last_rtt.unwrap_or(DEFAULT_RTT) + DEFAULT_BACKOFF_TIME) * 2;
        (avg_packet_bits / response_time.as_secs_f64()).max(MIN_NEAR_MAX_INCREASE_BPS_PER_S)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Overuse,
    Underuse,
    Normal,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    Increase = 1,
    Hold = 0,
    #[default]
    Decrease = -1,
}

impl State {
    fn transition(&self, signal: Signal) -> Self {
        let new_state = match (self, signal) {
            (_, Signal::Overuse) => Self::Decrease,
            // For (Decrease, Normal):
            // https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02#section-6 says to
            // transition to Hold here, but libWebRTC stays in decrease. We will eventually
            // transition to Hold on Underuse.
            (_, Signal::Underuse) | (Self::Decrease, Signal::Normal) => Self::Hold,
            (Self::Hold | Self::Increase, Signal::Normal) => Self::Increase,
        };

        if new_state != *self {
            log_rate_control_state!(new_state as i8);
        }

        new_state
    }
}

impl From<BandwidthUsage> for Signal {
    fn from(value: BandwidthUsage) -> Self {
        match value {
            BandwidthUsage::Overuse => Signal::Overuse,
            BandwidthUsage::Normal => Signal::Normal,
            BandwidthUsage::Underuse => Signal::Underuse,
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            State::Hold => write!(f, "hold"),
            State::Increase => write!(f, "increase"),
            State::Decrease => write!(f, "decrease"),
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Signal::Overuse => write!(f, "overuse"),
            Signal::Underuse => write!(f, "underuse"),
            Signal::Normal => write!(f, "normal"),
        }
    }
}

#[cfg(test)]
#[path = "rate_control_test.rs"]
mod test;
