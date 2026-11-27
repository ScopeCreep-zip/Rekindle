#[test]
fn latest_is_the_newest_evidence_from_any_peer() {
    let ledger = super::MediaLiveness::default();
    assert_eq!(ledger.latest(), None);
    ledger.note("a", 1_000);
    ledger.note("b", 3_000);
    ledger.note("a", 2_000);
    assert_eq!(ledger.latest(), Some(3_000));
}

use super::*;

#[test]
fn noted_peer_is_live_within_window() {
    let liveness = MediaLiveness::default();
    liveness.note("alice", 1_000);
    let live = liveness.live_within(MEDIA_LIVE_WINDOW_MS, 25_000);
    assert!(live.contains("alice"));
    assert_eq!(live.len(), 1);
}

#[test]
fn window_boundary_is_inclusive() {
    let liveness = MediaLiveness::default();
    liveness.note("alice", 1_000);
    assert!(liveness
        .live_within(MEDIA_LIVE_WINDOW_MS, 1_000 + MEDIA_LIVE_WINDOW_MS)
        .contains("alice"));
    assert!(!liveness
        .live_within(MEDIA_LIVE_WINDOW_MS, 1_000 + MEDIA_LIVE_WINDOW_MS + 1)
        .contains("alice"));
}

#[test]
fn re_note_refreshes_the_entry() {
    let liveness = MediaLiveness::default();
    liveness.note("alice", 1_000);
    liveness.note("alice", 40_000);
    // Stale by the first stamp, fresh by the refresh.
    assert!(liveness
        .live_within(MEDIA_LIVE_WINDOW_MS, 50_000)
        .contains("alice"));
}

#[test]
fn entry_ahead_of_now_reads_as_fresh() {
    let liveness = MediaLiveness::default();
    liveness.note("alice", 10_000);
    // A query racing the note with a slightly older clock reading
    // must not underflow into "stale".
    assert!(liveness
        .live_within(MEDIA_LIVE_WINDOW_MS, 9_500)
        .contains("alice"));
}

#[test]
fn note_prunes_entries_past_the_bound() {
    let liveness = MediaLiveness::default();
    liveness.note("old", 0);
    liveness.note("young", PRUNE_AFTER_MS + 1);
    // An unbounded window sees everything still in the map: the
    // old entry is gone (pruned), not merely outside the window.
    let all = liveness.live_within(u64::MAX, PRUNE_AFTER_MS + 1);
    assert!(!all.contains("old"));
    assert!(all.contains("young"));
}

#[test]
fn note_keeps_entries_within_the_prune_bound() {
    let liveness = MediaLiveness::default();
    liveness.note("kept", 0);
    liveness.note("young", PRUNE_AFTER_MS);
    let all = liveness.live_within(u64::MAX, PRUNE_AFTER_MS);
    assert!(all.contains("kept"));
    assert!(all.contains("young"));
}
