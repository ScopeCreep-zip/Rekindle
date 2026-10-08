//! The probe strategies of the probe controller (str0m `src/bwe/probe/control.rs`,
//! `impl ProbeControl`: initial, exponential, ALR, stagnant and large-drop probes),
//! moved into this file to stay under the workspace's 600-line cap.

use std::time::Instant;

use super::{
    LargeDrop, ProbeControl, BITRATE_DROP_THRESHOLD, BITRATE_DROP_TIMEOUT,
    MAX_PROBE_BITRATE_FACTOR, MAX_WAITING_TIME_FOR_PROBING_RESULT, MIN_TIME_BETWEEN_ALR_PROBES,
    MIN_TIME_BETWEEN_STAGNANT_PROBES, PROBE_FRACTION_AFTER_DROP, STAGNANT_PROBE_SCALE,
};
use crate::bwe::ProbeKind;
use crate::Bitrate;

impl ProbeControl {
    pub(super) fn maybe_initial(
        &mut self,
        now: Instant,
        desired: Bitrate,
        estimate: Bitrate,
    ) -> bool {
        // Initial probes only fire once at startup.
        if self.last_probe.is_some() {
            return false;
        }

        // Queue 3× and 6× of estimate.
        let p1 = estimate * self.config.first_exponential_probe_scale;
        let p2 = estimate * self.config.second_exponential_probe_scale;

        self.queue_probe(p1, ProbeKind::Initial, desired, now);
        self.queue_probe(p2, ProbeKind::Initial, desired, now);
        true
    }

    pub(super) fn maybe_exponential(
        &mut self,
        now: Instant,
        desired: Bitrate,
        estimate: Bitrate,
    ) -> bool {
        // Wait for pending probes to be dispatched first.
        if !self.pending.is_empty() {
            return false;
        }

        // Need a previous probe to continue from.
        let Some(last) = self.last_probe else {
            return false;
        };

        // Estimate must exceed 70% of last probe rate to trigger further probing.
        if estimate < last.further {
            return false;
        }

        let is_same = Some(estimate) == last.was_estimate;
        let time_since = self.time_since_last_probe(now);

        // Don't re-probe at the same estimate; wait for new result or timeout.
        if is_same && time_since < MAX_WAITING_TIME_FOR_PROBING_RESULT {
            return false;
        }

        let scale = self.last_cause.probe_scale(&self.config);
        let target = estimate * scale;

        // Already probed at max rate; no point probing again.
        let max = desired * MAX_PROBE_BITRATE_FACTOR;
        if target >= max && last.further >= max * self.config.further_probe_threshold {
            return false;
        }

        self.queue_probe(target, ProbeKind::Exponential, desired, now);

        true
    }

    pub(super) fn maybe_increase_alr(
        &mut self,
        now: Instant,
        desired: Bitrate,
        estimate: Bitrate,
    ) -> bool {
        // Don't interfere with initial probing phase.
        if self.is_during_initial(now) {
            return false;
        }

        // Allocation probes only fire in ALR (application-limited region).
        if !self.in_alr() {
            return false;
        }

        let prev = self.prev_desired;
        self.prev_desired = Some(desired);

        // Need a previous desired value to compare against.
        let Some(prev) = prev else {
            return false;
        };

        // Only probe if desired increased
        if desired <= prev {
            return false;
        }

        // No point probing if we already have enough bandwidth.
        if desired <= estimate {
            return false;
        }

        // Allocation probes at 1× and 2× of desired, capped by 2× estimate
        let current_bwe_limit = estimate * self.config.allocation_probe_limit_by_current_scale;

        let p1 = (desired * self.config.first_allocation_probe_scale).min(current_bwe_limit);
        self.queue_probe(p1, ProbeKind::IncreaseAlr, desired, now);

        let p2 = desired * self.config.second_allocation_probe_scale;
        if p2 <= current_bwe_limit && p2 > p1 {
            self.queue_probe(p2, ProbeKind::IncreaseAlr, desired, now);
        }

        true
    }

    pub(super) fn maybe_periodic_alr(&mut self, now: Instant, desired: Bitrate) -> bool {
        if !self.config.periodic_alr_probing {
            return false;
        }

        // Don't interfere with initial probing phase.
        if self.is_during_initial(now) {
            return false;
        }

        // Periodic probes only fire in ALR (application-limited region).
        if !self.in_alr() {
            return false;
        }

        // Respect minimum interval between ALR probes.
        if self.time_since_last_probe(now) < MIN_TIME_BETWEEN_ALR_PROBES {
            return false;
        }

        // Periodic ALR probe at 2× desired (capped by queue_probe to 2× desired anyway).
        // Using desired rather than estimate allows discovering higher capacity when
        // the app wants more bandwidth than currently estimated.
        let target = desired * self.config.further_exponential_probe_scale;
        self.queue_probe(target, ProbeKind::PeriodicAlr, desired, now);
        true
    }

    /// Probe when estimate has stagnated (no change for 15+ seconds) despite unmet demand.
    ///
    /// ## Why This Exists (str0m Addition)
    ///
    /// This probe type addresses a deadlock scenario in the BWE system where AIMD recovery
    /// cannot make progress after network capacity is restored:
    ///
    /// **The Deadlock:**
    /// 1. Network degrades from 5 Mbps → 1 Mbps, estimate drops to ~900 kbps
    /// 2. Application reduces send rate to ~500 kbps (below estimate)
    /// 3. Network recovers to 5 Mbps
    /// 4. AIMD tries to increase but is capped at 1.5× observed throughput:
    ///    500 kbps × 1.5 = 750 kbps maximum
    /// 5. Sending at 500 kbps = 71% of estimate, which is above ALR threshold (65%)
    /// 6. ALR never triggers → no periodic probing
    /// 7. Large-drop probe requires ALR or recent ALR exit (see `maybe_large_drop`)
    /// 8. System is stuck: estimate ~700 kbps on a 5 Mbps network
    ///
    /// **AIMD's 1.5× Cap (line 191 in rate_control.rs):**
    /// `observed_bitrate * 1.5 + Bitrate::kbps(10)`
    /// This prevents runaway growth beyond actual sending rate. It's conservative but
    /// necessary - without it, the estimate could grow unbounded even when we're barely
    /// sending anything.
    ///
    /// **ALR Detection Threshold:**
    /// ALR triggers when sending < 65% of estimate consistently for 500ms with budget
    /// accumulation > 80%. At 60-70% send rate, you're in the deadlock zone: too high
    /// to trigger ALR, too low for AIMD to help much.
    ///
    /// **Loss Controller's 1.5× Cap:**
    /// The loss controller also applies a 1.5× cap during recovery (line 303 in
    /// loss_controller.rs), compounding the AIMD limitation.
    ///
    /// ## How This Differs from WebRTC
    ///
    /// WebRTC does not have stagnation-based probing. They rely on:
    /// 1. Large-drop recovery probe (requires ALR or recent ALR exit)
    /// 2. Rapid recovery field trial (`WebRTC-BweRapidRecoveryExperiment`) which removes
    ///    the ALR requirement from large-drop probes
    ///
    /// str0m adds stagnant probing as a complementary mechanism that:
    /// - Catches deadlock regardless of whether a drop was detected
    /// - Provides periodic escape from any stagnation scenario, not just post-drop
    /// - Uses a conservative 15-second wait to avoid probing at convergence
    /// - Rate-limited to once per 30 seconds to prevent oscillation
    pub(super) fn maybe_stagnant(
        &mut self,
        now: Instant,
        desired: Bitrate,
        estimate: Bitrate,
    ) -> bool {
        // Don't interfere with initial probing phase.
        if self.is_during_initial(now) {
            return false;
        }

        // Don't probe in ALR (periodic ALR handles that).
        if self.in_alr() {
            return false;
        }

        let Some(last_change) = self.last_estimate_change else {
            return false;
        };

        if now.saturating_duration_since(last_change) < MIN_TIME_BETWEEN_STAGNANT_PROBES {
            return false;
        }

        // Only if there's unmet demand.
        if desired <= estimate {
            return false;
        }

        // Rate limit: at least 30 seconds between stagnation probes.
        if let Some(last_probe) = self.last_stagnant {
            if now.saturating_duration_since(last_probe) < MIN_TIME_BETWEEN_STAGNANT_PROBES {
                return false;
            }
        }

        // Probe at 2× estimate (conservative, won't overwhelm if at capacity).
        let probe_rate = estimate * STAGNANT_PROBE_SCALE;
        self.queue_probe(probe_rate, ProbeKind::Stagnant, desired, now);
        self.last_stagnant = Some(now);

        true
    }

    pub(super) fn maybe_large_drop(
        &mut self,
        now: Instant,
        desired: Bitrate,
        estimate: Bitrate,
    ) -> bool {
        // Don't interfere with initial probing phase.
        if self.is_during_initial(now) {
            return false;
        }

        // Detect large drops: estimate fell below 66% of previous.
        if self.large_drop.is_none() {
            if let Some(prev) = self.prev_estimate {
                if estimate < prev * BITRATE_DROP_THRESHOLD {
                    self.large_drop = Some(LargeDrop {
                        when: now,
                        bitrate_before: prev,
                    });
                }
            }
        }

        // No large drop detected.
        let Some(drop) = &self.large_drop else {
            return false;
        };

        // Drop expires after 5 seconds.
        if now.saturating_duration_since(drop.when) > BITRATE_DROP_TIMEOUT {
            self.large_drop = None;
            return false;
        }

        // Large-drop probing requires ALR context (in ALR or recently exited).
        if !self.in_alr() && !self.alr_ended_recently(now) {
            return false;
        }

        // Respect minimum interval between ALR probes.
        if self.time_since_last_probe(now) < MIN_TIME_BETWEEN_ALR_PROBES {
            return false;
        }

        // Probe at 85% of pre-drop bitrate.
        let target = drop.bitrate_before * PROBE_FRACTION_AFTER_DROP;
        self.queue_probe(target, ProbeKind::LargeDrop, desired, now);

        self.large_drop = None;
        true
    }
}
