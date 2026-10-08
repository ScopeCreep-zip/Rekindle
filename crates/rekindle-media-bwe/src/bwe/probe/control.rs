//! Bandwidth probing controller - decides when and how to probe network capacity.
//!
//! This module implements WebRTC's `ProbeController` state machine for discovering available
//! bandwidth through intentional bursts of packets at rates higher than current estimates.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::{ProbeClusterConfig, ProbeKind};
use crate::util::{already_happened, not_happening};
use crate::{Bitrate, TwccClusterId};

mod strategies;

// Port notes:
// This module ports WebRTC's `ProbeController` behavior from:
// `webrtc/modules/congestion_controller/goog_cc/probe_controller.cc`
//
// Key integration difference: WebRTC returns vectors of probe clusters, while str0m
// returns a single `ProbeClusterConfig` per `handle_timeout()` call. Configs are queued
// internally and `poll_timeout()` returns `already_happened()` until the queue is drained.

/// WebRTC: `kMaxWaitingTimeForProbingResult`.
const MAX_WAITING_TIME_FOR_PROBING_RESULT: Duration = Duration::from_secs(1);

/// WebRTC: `kBitrateDropThreshold`, `kBitrateDropTimeout`, `kProbeFractionAfterDrop`,
/// `kProbeUncertainty`, `kAlrEndedTimeout`, `kMinTimeBetweenAlrProbes`.
const BITRATE_DROP_THRESHOLD: f64 = 0.66;
const BITRATE_DROP_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_FRACTION_AFTER_DROP: f64 = 0.85;
const PROBE_UNCERTAINTY: f64 = 0.05;
const ALR_ENDED_TIMEOUT: Duration = Duration::from_secs(3);
const MIN_TIME_BETWEEN_ALR_PROBES: Duration = Duration::from_secs(5);

/// WebRTC: inline `* 2` in probe_controller.cc InitiateProbing().
/// Allows probing up to 2x max_bitrate to account for bursty streams.
const MAX_PROBE_BITRATE_FACTOR: f64 = 2.0;

/// Minimum time between stagnant periodic probes to avoid excessive probing when at capacity.
const MIN_TIME_BETWEEN_STAGNANT_PROBES: Duration = Duration::from_secs(15);

/// Threshold for considering an estimate change significant (5%).
const ESTIMATE_CHANGE_THRESHOLD: f64 = 0.05;

/// Probe rate scale for stagnation probes (2× current estimate).
const STAGNANT_PROBE_SCALE: f64 = 2.0;

/// WebRTC's `BandwidthLimitedCause` (subset used by probing gating).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandwidthLimitedCause {
    LossLimitedBweIncreasing,
    LossLimitedBwe,
    DelayBasedLimited,
    DelayBasedLimitedDelayIncreased,
}

pub struct ProbeControl {
    config: Config,
    next_timeout: Instant,
    enabled: bool,

    desired_bitrate: Option<Bitrate>,
    prev_desired: Option<Bitrate>,

    last_estimate: Option<Bitrate>,
    last_estimate_change: Option<Instant>,
    last_cause: BandwidthLimitedCause,

    prev_estimate: Option<Bitrate>,

    alr_start: Option<Instant>,
    alr_stop: Option<Instant>,

    last_probe: Option<LastProbe>,

    large_drop: Option<LargeDrop>,

    last_stagnant: Option<Instant>,

    next_cluster_id: TwccClusterId,
    pending: VecDeque<ProbeClusterConfig>,

    scheduled_exponential: Option<Instant>,
    scheduled_periodic_alr: Option<Instant>,
    scheduled_stagnant: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LastProbe {
    when: Instant,
    kind: ProbeKind,
    further: Bitrate,
    was_estimate: Option<Bitrate>,
}

struct LargeDrop {
    when: Instant,
    bitrate_before: Bitrate,
}

impl Default for ProbeControl {
    fn default() -> Self {
        Self {
            config: Config::default(),
            enabled: false,
            next_timeout: not_happening(),
            desired_bitrate: None,
            prev_desired: None,
            last_estimate: None,
            last_estimate_change: None,
            last_cause: BandwidthLimitedCause::DelayBasedLimited,
            prev_estimate: None,
            alr_start: None,
            alr_stop: None,
            next_cluster_id: 0.into(),
            last_probe: None,
            large_drop: None,
            last_stagnant: None,
            pending: VecDeque::new(),
            scheduled_exponential: None,
            scheduled_periodic_alr: None,
            scheduled_stagnant: None,
        }
    }
}

impl ProbeControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enable(&mut self, v: bool) {
        if !self.enabled && v {
            self.enabled = true;
            self.request_immediate();
        } else if self.enabled && !v {
            self.enabled = false;
            self.pending.clear();
            self.last_estimate = None;
            self.desired_bitrate = None;
            self.last_estimate_change = None;
            self.last_stagnant = None;
            self.last_probe = None;
            self.prev_estimate = None;
            self.scheduled_exponential = None;
            self.scheduled_periodic_alr = None;
            self.scheduled_stagnant = None;
            self.next_timeout = not_happening();
        }
    }

    pub fn set_desired_bitrate(&mut self, v: Bitrate) {
        // Don't accept Bitrate::ZERO as first ever value.
        if self.desired_bitrate.is_none() && v.is_zero() {
            return;
        }
        self.desired_bitrate = Some(v);
        self.request_immediate();
    }

    pub fn set_estimated_bitrate(&mut self, v: Bitrate, cause: BandwidthLimitedCause) {
        // Don't accept Bitrate::ZERO as first ever value.
        if self.last_estimate.is_none() && v.is_zero() {
            return;
        }

        // Check if estimate changed significantly (>5%) or cause changed.
        let dominated_by_last = self.last_estimate.is_some_and(|last| {
            let upper = last * (1.0 + ESTIMATE_CHANGE_THRESHOLD);
            let lower = last * (1.0 - ESTIMATE_CHANGE_THRESHOLD);
            v <= upper && v >= lower
        });

        if dominated_by_last && self.last_cause == cause {
            return;
        }

        self.last_estimate = Some(v);
        self.last_cause = cause;
        self.request_immediate();
    }

    pub fn set_alr_start_time(&mut self, t: Instant) {
        if self.alr_start.is_some() {
            return;
        }
        self.alr_start = Some(t);
        self.alr_stop = None;
        self.request_immediate();
    }

    pub fn set_alr_stop_time(&mut self, t: Instant) {
        if self.alr_start.is_none() || self.alr_stop.is_some() {
            return;
        }
        self.alr_start = None;
        self.alr_stop = Some(t);
        self.request_immediate();
    }

    fn request_immediate(&mut self) {
        self.next_timeout = already_happened();
        self.scheduled_exponential = None;
        self.scheduled_periodic_alr = None;
        self.scheduled_stagnant = None;
    }

    pub fn poll_timeout(&self) -> Instant {
        self.next_timeout
    }

    pub fn handle_timeout(&mut self, now: Instant) -> Option<ProbeClusterConfig> {
        // Spurious call before timeout is due - ignore.
        if now < self.next_timeout {
            return None;
        }

        // Timeout fired - reset to not_happening until we compute the next one.
        self.next_timeout = not_happening();

        // Probing is disabled until first packet sent and padding queue exists.
        if !self.enabled {
            return None;
        }

        // We need to have both desired AND last_estimate set to
        // start considering probing.
        let desired = self.desired_bitrate?;
        let estimate = self.last_estimate?;

        // Return pending probes first.
        if let Some(config) = self.pending.pop_front() {
            // Schedule another.
            self.request_immediate();
            return Some(config);
        }

        // Can't probe in certain bandwidth-limited states.
        if !self.can_probe(estimate) {
            return None;
        }

        // Try each probe type in order - only one fires per timeout.
        let _ = self.maybe_initial(now, desired, estimate)
            || self.maybe_exponential(now, desired, estimate)
            || self.maybe_increase_alr(now, desired, estimate)
            || self.maybe_large_drop(now, desired, estimate)
            || self.maybe_periodic_alr(now, desired)
            || self.maybe_stagnant(now, desired, estimate);

        self.update_estimate_change(now, estimate);

        // Update prev_estimate for next cycle (used by large drop and stagnation detection).
        self.prev_estimate = Some(estimate);

        // Update timeout based on current state.
        self.next_timeout = self.compute_next_timeout(now);

        if !self.pending.is_empty() {
            self.request_immediate();
        }

        self.pending.pop_front()
    }

    fn update_estimate_change(&mut self, now: Instant, estimate: Bitrate) {
        // Track when estimate last changed significantly (>5%).
        if let Some(prev) = self.prev_estimate {
            if estimate != prev {
                self.last_estimate_change = Some(now);
            }
        }

        // Initialize baseline if not set yet.
        if self.last_estimate_change.is_none() {
            self.last_estimate_change = Some(now);
        }
    }

    fn queue_probe(&mut self, bitrate: Bitrate, kind: ProbeKind, desired: Bitrate, now: Instant) {
        // Cap at 2× desired bitrate.
        let max = desired * MAX_PROBE_BITRATE_FACTOR;
        let bitrate = bitrate.min(max);

        // No probe at too small values.
        if bitrate < Bitrate::kbps(5) {
            return;
        }

        let cluster_id = self.next_cluster_id.inc();

        let config = ProbeClusterConfig::new(cluster_id, bitrate, kind)
            .with_min_packet_count(self.config.min_probe_packets_sent)
            .with_duration(self.config.min_probe_duration)
            .with_min_probe_delta(self.config.min_probe_delta);

        // Threshold for further exponential probing (probe_bitrate * 0.7).
        let probe_further = bitrate * self.config.further_probe_threshold;

        self.pending.push_back(config);
        self.last_probe = Some(LastProbe {
            when: now,
            kind,
            further: probe_further,
            was_estimate: self.last_estimate,
        });
    }

    fn compute_next_timeout(&mut self, now: Instant) -> Instant {
        // Exponential probing: wait for probe result before re-probing at same estimate.
        // This handles the case where we sent a probe but haven't received updated estimate yet.
        if let Some(last) = &self.last_probe {
            if matches!(last.kind, ProbeKind::Initial | ProbeKind::Exponential) {
                if self.scheduled_exponential.is_none() {
                    self.scheduled_exponential = Some(now + MAX_WAITING_TIME_FOR_PROBING_RESULT);
                }
                return self.scheduled_exponential.unwrap();
            }
        }

        // ALR periodic probing
        if self.config.periodic_alr_probing && self.in_alr() {
            if self.scheduled_periodic_alr.is_none() {
                self.scheduled_periodic_alr = Some(now + MIN_TIME_BETWEEN_ALR_PROBES);
            }
            return self.scheduled_periodic_alr.unwrap();
        }

        // Stagnant probing (only when not in ALR)
        if !self.in_alr() {
            if self.scheduled_stagnant.is_none() {
                self.scheduled_stagnant = Some(now + MIN_TIME_BETWEEN_STAGNANT_PROBES);
            }
            return self.scheduled_stagnant.unwrap();
        }

        not_happening()
    }

    fn can_probe(&self, estimate: Bitrate) -> bool {
        // Infinite estimate indicates no valid measurement yet.
        if estimate == Bitrate::INFINITY {
            return false;
        }

        // Only probe when delay-limited or loss-limited-but-increasing.
        // Don't probe during active congestion (loss-limited, delay-increased).
        matches!(
            self.last_cause,
            BandwidthLimitedCause::LossLimitedBweIncreasing
                | BandwidthLimitedCause::DelayBasedLimited
        )
    }

    fn in_alr(&self) -> bool {
        self.alr_start.is_some() && self.alr_stop.is_none()
    }

    fn alr_ended_recently(&self, now: Instant) -> bool {
        self.alr_stop
            .is_some_and(|stop| now.saturating_duration_since(stop) < ALR_ENDED_TIMEOUT)
    }

    fn is_during_initial(&self, now: Instant) -> bool {
        let is_initial = matches!(
            self.last_probe.map(|p| p.kind),
            Some(ProbeKind::Initial | ProbeKind::Exponential)
        );
        is_initial && self.time_since_last_probe(now) <= MAX_WAITING_TIME_FOR_PROBING_RESULT
    }

    fn last_when(&self) -> Option<Instant> {
        self.last_probe.map(|p| p.when)
    }

    fn time_since_last_probe(&self, now: Instant) -> Duration {
        self.last_when()
            .map_or(Duration::MAX, |t| now.saturating_duration_since(t))
    }
}

/// Configuration using WebRTC default constants (no field-trial plumbing).
#[derive(Debug, Clone, Copy)]
struct Config {
    // Initial/exponential probing
    first_exponential_probe_scale: f64,   // p1 = 3.0
    second_exponential_probe_scale: f64,  // p2 = 6.0
    further_exponential_probe_scale: f64, // step_size = 2.0
    further_probe_threshold: f64,         // 0.7

    // Allocation probing
    first_allocation_probe_scale: f64,            // 1.0
    second_allocation_probe_scale: f64,           // 2.0
    allocation_probe_limit_by_current_scale: f64, // 2.0

    // Probe cluster config defaults
    min_probe_packets_sent: usize, // 5
    min_probe_duration: Duration,  // 15ms
    min_probe_delta: Duration,     // 2ms

    // Gating / limits
    loss_limited_probe_scale: f64, // 1.5

    /// Periodic probes while application-limited. Off, as in libwebrtc,
    /// where `RateControlSettings::alr_probing` defaults to false and only
    /// screenshare field trials turn it on
    /// (`rtc_base/experiments/rate_control_settings.h`). str0m always
    /// probes; on a Veilid route each probe overshot the sustainable rate
    /// and the call oscillated (plan E4.3.3).
    periodic_alr_probing: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            first_exponential_probe_scale: 3.0,
            second_exponential_probe_scale: 6.0,
            further_exponential_probe_scale: 2.0,
            further_probe_threshold: 0.7,

            first_allocation_probe_scale: 1.0,
            second_allocation_probe_scale: 2.0,
            allocation_probe_limit_by_current_scale: 2.0,

            min_probe_packets_sent: 5,
            min_probe_duration: Duration::from_millis(15),
            min_probe_delta: Duration::from_millis(2),

            loss_limited_probe_scale: 1.5,

            periodic_alr_probing: false,
        }
    }
}

impl BandwidthLimitedCause {
    /// Probe scale factor for exponential probing.
    ///
    /// When loss-limited but increasing, use a more conservative 1.575× (1.5 * 1.05).
    /// Otherwise use the standard 2× scale.
    fn probe_scale(&self, config: &Config) -> f64 {
        match self {
            BandwidthLimitedCause::LossLimitedBweIncreasing => {
                config.loss_limited_probe_scale * (1.0 + PROBE_UNCERTAINTY)
            }
            _ => config.further_exponential_probe_scale,
        }
    }
}

#[cfg(test)]
mod test;
