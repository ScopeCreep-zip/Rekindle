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
