use super::*;

#[test]
fn a_window_counts_both_ends_and_resets() {
    let stats = FeedbackStats::default();
    stats.note_built("a");
    stats.note_built("a");
    stats.note_handed(
        "a",
        Duration::from_millis(40),
        Duration::from_millis(3),
        true,
    );
    stats.note_handed(
        "a",
        Duration::from_millis(900),
        Duration::from_millis(5),
        false,
    );
    stats.note_accepted("a");
    stats.note_unknown_peer();
    let w = stats.take_window();
    assert_eq!(w.peers.len(), 1);
    let a = &w.peers[0];
    assert_eq!((a.built, a.handed, a.failed, a.accepted), (2, 1, 1, 1));
    assert_eq!(a.lock_wait_ms, (40, 40, 900));
    assert_eq!(w.unknown_peer, 1);
    assert_eq!(stats.take_window(), FeedbackWindow::default());
}
