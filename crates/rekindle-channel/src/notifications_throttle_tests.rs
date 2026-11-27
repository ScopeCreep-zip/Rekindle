use super::*;

#[test]
fn first_five_attempts_emit_normally() {
    let throttle = NotificationThrottle::new();
    for i in 0_u128..5 {
        assert_eq!(
            throttle.record_attempt("c", "ch", 1000 + i * 100),
            NotificationDecision::Emit,
            "burst slot {i} should emit",
        );
    }
}

#[test]
fn sixth_attempt_inside_window_drops() {
    let throttle = NotificationThrottle::new();
    for i in 0_u128..5 {
        throttle.record_attempt("c", "ch", 1000 + i * 100);
    }
    assert_eq!(
        throttle.record_attempt("c", "ch", 1500),
        NotificationDecision::Drop,
    );
}

#[test]
fn next_attempt_after_window_emits_summary() {
    let throttle = NotificationThrottle::new();
    for i in 0_u128..5 {
        throttle.record_attempt("c", "ch", 1000 + i * 100);
    }
    throttle.record_attempt("c", "ch", 1500);
    throttle.record_attempt("c", "ch", 1600);
    assert_eq!(
        throttle.record_attempt("c", "ch", 12_000),
        NotificationDecision::EmitSummary { bundled_count: 3 },
    );
    assert_eq!(
        throttle.record_attempt("c", "ch", 12_100),
        NotificationDecision::Emit,
    );
}

#[test]
fn separate_channels_have_separate_windows() {
    let throttle = NotificationThrottle::new();
    for i in 0_u128..5 {
        throttle.record_attempt("c", "ch_a", 1000 + i * 10);
    }
    assert_eq!(
        throttle.record_attempt("c", "ch_b", 1100),
        NotificationDecision::Emit,
    );
    assert_eq!(
        throttle.record_attempt("c", "ch_a", 1100),
        NotificationDecision::Drop,
    );
}
