use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

struct Manual(AtomicU64);

impl Clock for Manual {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
fn a_wall_clock_jump_between_ticks_is_a_sleep() {
    let clock = Arc::new(Manual(AtomicU64::new(1_000_000)));
    let mut detector = WakeDetector::new(Arc::clone(&clock) as Arc<dyn Clock>);
    clock.0.fetch_add(5_000, Ordering::SeqCst);
    assert!(!detector.tick(), "an on-time tick");
    clock.0.fetch_add(29_999, Ordering::SeqCst);
    assert!(!detector.tick(), "under the gap");
    clock.0.fetch_add(30_000, Ordering::SeqCst);
    assert!(detector.tick(), "a sleep");
    clock.0.fetch_sub(60_000, Ordering::SeqCst);
    assert!(!detector.tick(), "a clock set back is no sleep");
}
