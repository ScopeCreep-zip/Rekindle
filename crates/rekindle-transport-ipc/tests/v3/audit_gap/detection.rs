use rekindle_transport_ipc::v3::audit::gap::{GapDetector, GapEvent};

#[test]
fn no_gap_when_sequential() {
    let mut d = GapDetector::new();
    assert!(matches!(d.observe(0), GapEvent::InOrder));
    assert!(matches!(d.observe(1), GapEvent::InOrder));
    assert!(matches!(d.observe(2), GapEvent::InOrder));
    assert!(matches!(d.observe(3), GapEvent::InOrder));
}

#[test]
fn gap_detected_on_skip() {
    let mut d = GapDetector::new();
    d.observe(0);
    d.observe(1);
    let event = d.observe(3); // skipped 2
    match event {
        GapEvent::GapDetected { start, end } => {
            assert_eq!(start, 2);
            assert_eq!(end, 2);
        }
        other => panic!("Expected GapDetected, got {other:?}"),
    }
}

#[test]
fn multiple_gaps_detected() {
    let mut d = GapDetector::new();
    d.observe(0);
    d.observe(1);
    let gap1 = d.observe(4); // skipped 2, 3
    match gap1 {
        GapEvent::GapDetected { start, end } => {
            assert_eq!(start, 2);
            assert_eq!(end, 3);
        }
        other => panic!("Expected gap [2,3], got {other:?}"),
    }
    d.observe(5);
    let gap2 = d.observe(8); // skipped 6, 7
    match gap2 {
        GapEvent::GapDetected { start, end } => {
            assert_eq!(start, 6);
            assert_eq!(end, 7);
        }
        other => panic!("Expected gap [6,7], got {other:?}"),
    }
}

#[test]
fn gap_at_start() {
    let mut d = GapDetector::new();
    let event = d.observe(1); // skipped 0
    match event {
        GapEvent::GapDetected { start, end } => {
            assert_eq!(start, 0);
            assert_eq!(end, 0);
        }
        other => panic!("Expected gap at 0, got {other:?}"),
    }
}

#[test]
fn duplicate_seq_detected() {
    let mut d = GapDetector::new();
    d.observe(0);
    d.observe(1);
    let event = d.observe(1); // duplicate
    assert!(
        matches!(event, GapEvent::Duplicate { seq: 1 }),
        "Expected Duplicate, got {event:?}"
    );
}

#[test]
fn expected_next_starts_at_zero() {
    let d = GapDetector::new();
    assert_eq!(d.expected_next(), 0);
}

#[test]
fn expected_next_advances() {
    let mut d = GapDetector::new();
    d.observe(0);
    assert_eq!(d.expected_next(), 1);
    d.observe(1);
    assert_eq!(d.expected_next(), 2);
}

#[test]
fn expected_next_jumps_on_gap() {
    let mut d = GapDetector::new();
    d.observe(0);
    d.observe(5); // gap [1..4]
    assert_eq!(d.expected_next(), 6);
}

#[test]
fn gap_window_limit() {
    let mut d = GapDetector::with_max_gap(100);
    d.observe(0);
    let event = d.observe(102); // gap of 101 frames, exceeds limit of 100
    assert!(
        matches!(event, GapEvent::GapTooLarge { .. }),
        "Expected GapTooLarge, got {event:?}"
    );
}

#[test]
fn gap_within_window_limit_accepted() {
    let mut d = GapDetector::with_max_gap(100);
    d.observe(0);
    let event = d.observe(100); // gap of 99, within limit
    assert!(matches!(event, GapEvent::GapDetected { .. }));
}
