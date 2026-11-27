use std::time::{Duration, Instant};

use super::budget::IntervalBudget;
use crate::{Bitrate, DataSize};

/// Application Limited Region detector.
///
/// Detects when we're sending significantly less than network capacity,
/// using IntervalBudget to handle bursty encoder traffic over 500ms windows.
///
/// This is critical for real-world usage where encoders produce bursty traffic
/// (keyframes, scene changes, temporal layers, etc.) The IntervalBudget smooths
/// these bursts over a 500ms window to detect sustained application-limited state.
///
/// ## ALR State Transitions
///
/// - **Enter ALR**: When budget ratio > 0.80 (sustained low usage for 500ms)
/// - **Exit ALR**: When budget ratio < 0.50 (increased usage)
/// - The 30% hysteresis gap prevents rapid state flapping
#[derive(Debug)]
pub struct AlrDetector {
    /// Budget tracker with 500ms window
    budget: IntervalBudget,
    /// Current ALR state
    state: AlrState,
    /// Last time we sent bytes
    last_send_time: Option<Instant>,

    // Configuration matching libWebRTC defaults
    /// Target send rate as fraction of estimate (0.65 = 65%)
    bandwidth_usage_ratio: f64,
    /// Budget ratio threshold to enter ALR (0.80 = 80%)
    start_budget_level_ratio: f64,
    /// Budget ratio threshold to exit ALR (0.50 = 50%)
    stop_budget_level_ratio: f64,
}

/// ALR state for the detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlrState {
    /// Not currently in Application Limited Region
    NotInAlr,
    /// In Application Limited Region since the given instant
    InAlr(Instant),
}

impl AlrState {
    fn alr_start_time(&self) -> Option<Instant> {
        match self {
            AlrState::InAlr(start) => Some(*start),
            AlrState::NotInAlr => None,
        }
    }
}

impl AlrDetector {
    /// Create a new ALR detector with libWebRTC default configuration.
    pub fn new() -> Self {
        Self {
            // Start with zero bitrate, will be updated via set_estimated_bitrate()
            // Use can_build_up_underuse=true for ALR detection
            budget: IntervalBudget::new(Bitrate::ZERO, true),
            state: AlrState::NotInAlr,
            last_send_time: None,

            // libWebRTC defaults from alr_detector.cc:30-49
            bandwidth_usage_ratio: 0.65,
            start_budget_level_ratio: 0.80,
            stop_budget_level_ratio: 0.50,
        }
    }

    /// Advance the budget through complete media silence. Keep WebRTC's
    /// packet-driven updates while media is flowing: frequent zero-byte updates
    /// would round away fractional byte credit and change ALR transitions.
    pub fn handle_timeout(&mut self, now: Instant) {
        const IDLE_INTERVAL: Duration = Duration::from_millis(500);
        if self
            .last_send_time
            .is_none_or(|last| now.saturating_duration_since(last) >= IDLE_INTERVAL)
        {
            self.on_bytes_sent(DataSize::ZERO, now);
        }
    }

    /// Update with bytes sent.
    ///
    /// Should be called for every media packet sent (excluding padding and probes).
    /// This updates the IntervalBudget and checks for ALR state transitions.
    pub fn on_bytes_sent(&mut self, bytes: DataSize, now: Instant) {
        // First call - just record time
        let Some(last) = self.last_send_time else {
            self.last_send_time = Some(now);
            return;
        };

        // saturating_duration_since protects against time going backwards
        let delta = now.saturating_duration_since(last);
        self.last_send_time = Some(now);

        // Update budget based on time and usage
        self.budget.use_budget(bytes);
        self.budget.increase_budget(delta);

        // Check for state transitions with hysteresis
        let ratio = self.budget.budget_ratio();

        match self.state {
            AlrState::NotInAlr => {
                if ratio > self.start_budget_level_ratio {
                    // Enter ALR: budget accumulated because we're not sending much
                    self.state = AlrState::InAlr(now);
                    tracing::trace!(
                        "ALR: Entered ALR state (ratio={:.3} > threshold={:.3})",
                        ratio,
                        self.start_budget_level_ratio
                    );
                }
            }
            AlrState::InAlr(_) => {
                if ratio < self.stop_budget_level_ratio {
                    // Exit ALR: budget depleted because we're sending more
                    self.state = AlrState::NotInAlr;
                    tracing::trace!(
                        "ALR: Exited ALR state (ratio={:.3} < threshold={:.3})",
                        ratio,
                        self.stop_budget_level_ratio
                    );
                }
            }
        }
    }

    /// Update the BWE estimate.
    ///
    /// This adjusts the IntervalBudget's target rate to 65% of the estimate.
    /// Should be called whenever the bandwidth estimate changes.
    pub fn set_estimated_bitrate(&mut self, estimate: Bitrate) {
        let target = estimate * self.bandwidth_usage_ratio;
        self.budget.set_target_rate(target);
    }

    /// Get the time when ALR started, if currently in ALR.
    ///
    /// Returns `Some(Instant)` if in ALR, `None` otherwise.
    /// Used by ProbeControl to determine if periodic ALR probes should be sent.
    pub fn alr_start_time(&self) -> Option<Instant> {
        self.state.alr_start_time()
    }

    #[cfg(test)]
    pub fn budget_ratio(&self) -> f64 {
        self.budget.budget_ratio()
    }
}

#[cfg(test)]
#[path = "detector_tests.rs"]
mod tests;
