//! Interval budget tracker for ALR detection.
//!
//! Port of libWebRTC's IntervalBudget class from
//! `webrtc/modules/pacing/interval_budget.{h,cc}`

use std::time::Duration;

#[cfg(test)]
use crate::util::f64_to_i64;
use crate::util::i64_to_f64;
use crate::{Bitrate, DataSize};

/// Leaky bucket budget tracker over a fixed time window.
///
/// Used by ALR detector to handle bursty traffic (encoders, etc.)
/// over a 500ms window. Tracks how much "budget" we have left
/// based on target rate vs. actual send rate.
///
/// This implements the same logic as libWebRTC's IntervalBudget class
/// to ensure ALR detection behaves identically when handling bursty
/// encoder traffic (keyframes, scene changes, temporal layers, etc.)
#[derive(Debug, Clone)]
pub struct IntervalBudget {
    /// Target bitrate for budget calculation
    target_rate: Bitrate,
    /// Maximum bytes that can accumulate in the budget
    max_bytes_in_budget: DataSize,
    /// Current bytes remaining in budget (can be negative = debt)
    bytes_remaining: DataSize,
    /// Whether underuse can build up credit across intervals
    can_build_up_underuse: bool,
}

impl IntervalBudget {
    /// Window size for budget tracking
    const WINDOW: Duration = Duration::from_millis(500);

    /// Create a new IntervalBudget with the specified target rate.
    pub fn new(target_rate: Bitrate, can_build_up_underuse: bool) -> Self {
        let max_bytes_in_budget = Self::calculate_max_bytes(target_rate);

        Self {
            target_rate,
            max_bytes_in_budget,
            bytes_remaining: DataSize::ZERO,
            can_build_up_underuse,
        }
    }

    /// Update the target bitrate.
    ///
    /// Called when the BWE estimate changes. Clamps existing bytes_remaining
    /// to the new max_bytes_in_budget range.
    pub fn set_target_rate(&mut self, target_rate: Bitrate) {
        self.target_rate = target_rate;
        self.max_bytes_in_budget = Self::calculate_max_bytes(target_rate);

        // Clamp bytes_remaining to new budget limits
        let max = self.max_bytes_in_budget;
        let neg_max = max * -1i64;
        self.bytes_remaining = DataSize::bytes(
            self.bytes_remaining
                .as_bytes_i64()
                .clamp(neg_max, max.as_bytes_i64()),
        );
    }

    /// Add budget based on elapsed time.
    ///
    /// Increases the budget by `target_rate * delta_time`.
    /// Behavior depends on `can_build_up_underuse`:
    /// - If true: underuse accumulates (for ALR detection)
    /// - If false: budget resets each interval (for pacing)
    pub fn increase_budget(&mut self, delta_time: Duration) {
        let bytes = self.target_rate * delta_time;

        let max = self.max_bytes_in_budget;
        if self.bytes_remaining.as_bytes_i64() < 0 || self.can_build_up_underuse {
            // We overused last interval, compensate this interval
            // OR we allow building up underuse credit
            self.bytes_remaining = DataSize::bytes(
                (self.bytes_remaining + bytes)
                    .as_bytes_i64()
                    .min(max.as_bytes_i64()),
            );
        } else {
            // If we underused last interval we can't use it this interval
            self.bytes_remaining = DataSize::bytes(bytes.as_bytes_i64().min(max.as_bytes_i64()));
        }
    }

    /// Consume budget by the specified number of bytes.
    ///
    /// This represents sending data. The budget can go negative (debt).
    pub fn use_budget(&mut self, bytes: DataSize) {
        let max = self.max_bytes_in_budget;
        let neg_max = max * -1i64;
        self.bytes_remaining =
            DataSize::bytes((self.bytes_remaining - bytes).as_bytes_i64().max(neg_max));
    }

    /// Get the current budget ratio.
    ///
    /// Returns a value in the range [-1.0, 1.0]:
    /// - 1.0 = full budget (maximum underuse)
    /// - 0.0 = neutral (sending exactly at target rate)
    /// - -1.0 = maximum debt (maximum overuse)
    ///
    /// Used by ALR detector for hysteresis thresholds:
    /// - Enter ALR when ratio > 0.80 (sustained low usage)
    /// - Exit ALR when ratio < 0.50 (increased usage)
    pub fn budget_ratio(&self) -> f64 {
        let max = self.max_bytes_in_budget.as_bytes_i64();
        if max == 0 {
            return 0.0;
        }
        i64_to_f64(self.bytes_remaining.as_bytes_i64()) / i64_to_f64(max)
    }

    /// Calculate max_bytes_in_budget from target rate
    fn calculate_max_bytes(target_rate: Bitrate) -> DataSize {
        target_rate * Self::WINDOW
    }

    #[cfg(test)]
    pub fn target_rate(&self) -> Bitrate {
        self.target_rate
    }

    #[cfg(test)]
    pub fn bytes_remaining(&self) -> i64 {
        self.bytes_remaining.as_bytes_i64()
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
