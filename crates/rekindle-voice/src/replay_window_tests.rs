use super::*;

#[test]
fn first_packet_accepted() {
    let mut w = CtrWindow::new();
    assert!(w.check_and_insert(42));
    assert_eq!(w.high_water(), Some(42));
}

#[test]
fn duplicate_rejected_immediately() {
    let mut w = CtrWindow::new();
    assert!(w.check_and_insert(10));
    assert!(!w.check_and_insert(10));
}

#[test]
fn out_of_order_within_window_accepted_once() {
    let mut w = CtrWindow::new();
    assert!(w.check_and_insert(10));
    assert!(w.check_and_insert(20));
    // A reordered packet 15 within the window — accept once.
    assert!(w.check_and_insert(15));
    // Replay of 15 — reject.
    assert!(!w.check_and_insert(15));
}

#[test]
fn below_window_rejected() {
    let mut w = CtrWindow::new();
    assert!(w.check_and_insert(1000));
    // Sequence 1 is far below the window — reject (could not be a
    // late-but-novel reorder; window only protects 256 back).
    assert!(!w.check_and_insert(1));
}

#[test]
fn forward_jump_resets_far_window() {
    let mut w = CtrWindow::new();
    assert!(w.check_and_insert(10));
    // Big jump — old bits are all cleared, only the new high_water
    // bit is set.
    assert!(w.check_and_insert(10 + WINDOW_SIZE * 5));
    // Replay of 10 should still be rejected (it's now below window).
    assert!(!w.check_and_insert(10));
}

#[test]
fn replay_at_high_water_rejected() {
    let mut w = CtrWindow::new();
    assert!(w.check_and_insert(100));
    assert!(w.check_and_insert(101));
    // Replay of 101 (== current high_water) must be rejected.
    assert!(!w.check_and_insert(101));
}

#[test]
fn many_in_order_packets() {
    let mut w = CtrWindow::new();
    for seq in 0..10_000u64 {
        assert!(w.check_and_insert(seq), "seq {seq}");
    }
    // Verify replays of recent sequences are rejected.
    assert!(!w.check_and_insert(9_999));
    assert!(!w.check_and_insert(9_500));
    // And below-window sequences too.
    assert!(!w.check_and_insert(0));
}

#[test]
fn counters_beyond_u32_work() {
    let mut w = CtrWindow::new();
    let big = u64::from(u32::MAX) + 10;
    assert!(w.check_and_insert(big));
    assert!(!w.check_and_insert(big));
    assert!(w.check_and_insert(big - 1));
}

#[test]
fn window_edge_boundary() {
    let mut w = CtrWindow::new();
    assert!(w.check_and_insert(WINDOW_SIZE));
    // Sequence at exactly `WINDOW_SIZE - 1` ago = offset = WINDOW_SIZE - 1
    // is the oldest sequence still in the window — accept.
    assert!(w.check_and_insert(1));
    // Sequence at offset == WINDOW_SIZE — outside the window, reject.
    assert!(!w.check_and_insert(0));
}
