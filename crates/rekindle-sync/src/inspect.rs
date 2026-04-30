//! Inspect loop policy helpers.

use std::time::{Duration, Instant};

pub const INSPECT_INTERVAL: Duration = Duration::from_secs(60);

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
