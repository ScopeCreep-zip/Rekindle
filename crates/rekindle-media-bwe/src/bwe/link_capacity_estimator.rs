use std::time::{Duration, Instant};

use crate::Bitrate;

/// Estimates link capacity based on successful probe results during ALR.
///
/// This tracks the link's proven capacity from probes sent when the application
/// is in Application Limited Region (ALR), meaning the app is sending less than
/// network capacity. During ALR, probes can discover the true available bandwidth
/// without being constrained by application send rate.
///
/// The estimator only accepts probe results obtained while in ALR, as these
/// represent genuine capacity measurements. Probes during non-ALR periods
/// may be artificially limited by application sending patterns.
///
/// Capacity estimates decay over time (default 60s) since network conditions
/// can change, and old measurements become less reliable.
#[derive(Default)]
pub struct LinkCapacityEstimator {
    /// Current estimate of link capacity, if available
    capacity_estimate: Option<Bitrate>,

    /// Time when the capacity estimate was last updated
    last_estimate_time: Option<Instant>,
}

impl LinkCapacityEstimator {
    /// Default duration before capacity estimate resets (60 seconds)
    const DEFAULT_RESET_WINDOW: Duration = Duration::from_secs(60);

    /// Create a new LinkCapacityEstimator with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the capacity estimate from a successful probe result.
    ///
    /// This should only be called for probes that were sent during ALR.
    /// The estimate is stored along with the current time for decay tracking.
    pub fn update_from_probe(&mut self, probe_estimate: Bitrate, now: Instant) {
        // Only accept valid probe estimates
        if !probe_estimate.is_valid() {
            return;
        }

        // Update or set the capacity estimate
        // If we already have an estimate, take the max (capacity shouldn't decrease
        // from successful probes, only from decay/timeout)
        let current = self.capacity_estimate.get_or_insert(probe_estimate);
        *current = (*current).max(probe_estimate);

        self.last_estimate_time = Some(now);

        tracing::trace!(
            "Link capacity estimate updated to {} from probe",
            probe_estimate
        );
    }

    /// Get the current capacity estimate, if available and not expired.
    pub fn capacity_estimate(&self, now: Instant) -> Option<Bitrate> {
        let estimate = self.capacity_estimate?;
        let last_time = self.last_estimate_time?;

        // Check if estimate has expired (defensive against clock skew)
        if now.saturating_duration_since(last_time) > Self::DEFAULT_RESET_WINDOW {
            tracing::trace!("Link capacity estimate expired");
            return None;
        }

        Some(estimate)
    }

    /// Reset the capacity estimate.
    #[cfg(test)]
    pub fn reset(&mut self) {
        if self.capacity_estimate.is_some() {
            tracing::trace!("Link capacity estimate reset");
        }
        self.capacity_estimate = None;
        self.last_estimate_time = None;
    }

    /// Check if we currently have a valid capacity estimate
    #[cfg(test)]
    pub fn has_estimate(&self) -> bool {
        self.capacity_estimate.is_some() && self.last_estimate_time.is_some()
    }
}

#[cfg(test)]
#[path = "link_capacity_estimator_tests.rs"]
mod tests;
