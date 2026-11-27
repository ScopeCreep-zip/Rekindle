use super::*;

#[test]
fn the_clock_offset_cancels() {
    // The receiver's clock is 10 s ahead; audio plays 120 ms after
    // capture, video renders 180 ms after: video 60 ms late.
    let mut s = SyncTracker::new();
    for _ in 0..50 {
        s.note_audio(10_000 + 120);
        s.note_video(10_000 + 180);
    }
    assert_eq!(s.sample(), Some(60));
    assert!(within_bounds(60));
}

#[test]
fn needs_both_kinds_each_sample() {
    let mut s = SyncTracker::new();
    s.note_audio(100);
    assert_eq!(s.sample(), None);
    s.note_video(150);
    assert_eq!(s.sample(), Some(50));
    s.note_video(150);
    assert_eq!(s.sample(), None, "audio window started fresh");
}

#[test]
fn bounds_follow_p1305() {
    assert!(within_bounds(90));
    assert!(!within_bounds(91), "audio 91 ms ahead");
    assert!(within_bounds(-185));
    assert!(!within_bounds(-186), "audio 186 ms behind");
}

#[test]
fn summary_reports_spread_and_violations() {
    let mut s = SyncTracker::new();
    for skew in [10, 20, 30, 200, -300] {
        s.note_audio(0);
        s.note_video(skew);
        s.sample();
    }
    let sum = s.summary();
    assert_eq!(sum.samples, 5);
    assert_eq!(sum.skew_p50_ms, 20);
    assert_eq!(sum.outside_bounds, 2);
}
