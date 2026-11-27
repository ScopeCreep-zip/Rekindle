use super::*;

fn render_every(tracker: &mut RenderTracker, start: u64, step: u64, n: u64) -> u64 {
    let mut t = start;
    for _ in 0..n {
        tracker.on_rendered(t);
        t += step;
    }
    t - step
}

#[test]
fn steady_30fps_has_no_freeze() {
    let mut r = RenderTracker::new();
    render_every(&mut r, 0, 33, 300);
    let s = r.summary();
    assert_eq!(s.freeze_count, 0);
    assert_eq!(s.pause_count, 0);
    assert!((s.fps - 30.3).abs() < 0.1, "{}", s.fps);
}

#[test]
fn a_gap_of_avg_plus_150_is_a_freeze() {
    let mut r = RenderTracker::new();
    let last = render_every(&mut r, 0, 33, 60);
    // With this interval in the window the average is
    // (29·33 + 200)/30 = 38: threshold max(114, 188) = 188.
    r.on_rendered(last + 200);
    let s = r.summary();
    assert_eq!(s.freeze_count, 1);
    assert_eq!(s.total_freezes_ms, 200);
}

#[test]
fn a_gap_just_under_the_threshold_is_not() {
    let mut r = RenderTracker::new();
    let last = render_every(&mut r, 0, 33, 60);
    // avg over 30 with this interval x: (29·33 + x)/30; freeze needs
    // x ≥ avg + 150 → x ≥ 189.4.
    r.on_rendered(last + 185);
    assert_eq!(r.summary().freeze_count, 0);
}

#[test]
fn at_15fps_three_times_the_average_decides() {
    let mut r = RenderTracker::new();
    let last = render_every(&mut r, 0, 66, 60);
    // Average (29·66 + 230)/30 = 71: threshold max(213, 221) = 221.
    r.on_rendered(last + 230);
    assert_eq!(r.summary().freeze_count, 1);
}

#[test]
fn no_freeze_before_five_intervals() {
    let mut r = RenderTracker::new();
    r.on_rendered(0);
    r.on_rendered(33);
    r.on_rendered(66);
    r.on_rendered(1_000);
    assert_eq!(r.summary().freeze_count, 0);
}

#[test]
fn over_five_seconds_is_a_pause_not_a_freeze() {
    let mut r = RenderTracker::new();
    let last = render_every(&mut r, 0, 33, 60);
    let resumed = last + 6_000;
    r.on_rendered(resumed);
    render_every(&mut r, resumed + 33, 33, 30);
    let s = r.summary();
    assert_eq!(s.pause_count, 1);
    assert_eq!(s.total_pauses_ms, 6_000);
    assert_eq!(s.freeze_count, 0);
}

#[test]
fn harmonic_fps_penalises_stalls() {
    let mut smooth = RenderTracker::new();
    render_every(&mut smooth, 0, 50, 41);
    let mut stalled = RenderTracker::new();
    let last = render_every(&mut stalled, 0, 25, 21);
    stalled.on_rendered(last + 1_000);
    let a = smooth.summary();
    let b = stalled.summary();
    assert_eq!(a.rendered_span_ms, 2_000);
    assert_eq!(b.rendered_span_ms, 1_500);
    assert!(b.harmonic_fps < a.harmonic_fps / 5.0, "{b:?} vs {a:?}");
}

#[test]
fn post_fec_loss_counts_sequence_gaps() {
    let mut c = ReceiveCounters::default();
    for seq in [1, 2, 3, 6, 7] {
        c.on_completed(seq, seq == 7);
    }
    assert_eq!(c.completed, 5);
    assert_eq!(c.lost, 2);
    assert_eq!(c.recovered_by_fec, 1);
    // A late frame behind the highest fills nothing.
    c.on_completed(4, false);
    assert_eq!(c.lost, 2);
    assert!((c.loss_fraction() - 2.0 / 8.0).abs() < 1e-9);
}

#[test]
fn post_fec_loss_survives_sequence_wrap() {
    let mut c = ReceiveCounters::default();
    c.on_completed(u32::MAX - 1, false);
    c.on_completed(1, false);
    assert_eq!(c.lost, 2, "MAX and 0 were skipped");
}
