//! Inspect loop policy helpers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const INSPECT_INTERVAL: Duration = Duration::from_secs(60);

/// Process-wide miss-rate telemetry for the inspect catch-up path (A8).
///
/// The 60s `inspect_dht_record` poll is the backstop for missed watch
/// notifications ("message X isn't showing up"). 0.5.4 made watches
/// transaction-aware, which should make misses rarer — but
/// `INSPECT_INTERVAL` must only be relaxed against a MEASURED miss
/// rate, not optimism. Every inspection records whether it surfaced
/// subkey changes the watch path had not already delivered.
#[derive(Debug, Default)]
pub struct InspectTelemetry {
    /// Inspections where local sequences already matched the network
    /// (the poll was redundant — watches delivered everything).
    clean: AtomicU64,
    /// Inspections that surfaced changed subkeys (the watch path missed
    /// them; the poll earned its keep).
    missed: AtomicU64,
}

/// The process-wide [`InspectTelemetry`] instance. All inspect-path
/// callers (the 60s loop and on-demand background sync) record here.
pub static INSPECT_TELEMETRY: InspectTelemetry = InspectTelemetry {
    clean: AtomicU64::new(0),
    missed: AtomicU64::new(0),
};

impl InspectTelemetry {
    /// Record one inspection outcome. `surfaced_changes` = the report
    /// showed network sequences ahead of local (a watch miss).
    pub fn record(&self, surfaced_changes: bool) {
        if surfaced_changes {
            self.missed.fetch_add(1, Ordering::Relaxed);
        } else {
            self.clean.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Totals so far: `(clean, missed)`.
    pub fn counts(&self) -> (u64, u64) {
        (
            self.clean.load(Ordering::Relaxed),
            self.missed.load(Ordering::Relaxed),
        )
    }
}

#[derive(Debug, Clone)]
pub struct InspectLoop {
    interval: Duration,
    last_tick: Instant,
}

impl InspectLoop {
    pub fn new(now: Instant) -> Self {
        Self {
            interval: INSPECT_INTERVAL,
            last_tick: now,
        }
    }

    pub fn should_run_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_tick) >= self.interval
    }

    pub fn mark_ran(&mut self, now: Instant) {
        self.last_tick = now;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::InspectLoop;

    #[test]
    fn telemetry_counts_clean_and_missed() {
        // Use a local instance, not the global static, so the test is
        // isolated from other callers in the process.
        let t = super::InspectTelemetry::default();
        t.record(false);
        t.record(true);
        t.record(true);
        assert_eq!(t.counts(), (1, 2));
    }

    #[test]
    fn runs_every_sixty_seconds() {
        let start = Instant::now();
        let loop_state = InspectLoop::new(start);
        assert!(!loop_state.should_run_at(start + Duration::from_secs(59)));
        assert!(loop_state.should_run_at(start + Duration::from_secs(60)));
    }

    /// Three-path independence: when watches are inactive (Path 3 watch failed
    /// or unavailable), inspect polling is the sole consistency mechanism.
    /// The inspect loop fires at 60s, and GapDetector catches stale subkeys.
    #[test]
    fn inspect_catches_gaps_when_watches_inactive() {
        use crate::gap::GapDetector;
        use crate::watch::WatchManager;

        let start = Instant::now();
        let loop_state = InspectLoop::new(start);
        let wm = WatchManager::default(); // No watches registered

        // Watch is NOT active for this record — inspect is the only path
        assert!(!wm.is_active("channel_record_key"));

        // After 60s, inspect fires
        assert!(loop_state.should_run_at(start + Duration::from_secs(60)));

        // Inspect discovers gaps that gossip and watches missed
        let local = [0, 0, 0];
        let network = [3, 2, 0];
        let gaps = GapDetector::detect(&local, &network);
        assert_eq!(gaps.len(), 2);
        // Inspect can now fetch these subkeys via get_dht_value
    }
}
