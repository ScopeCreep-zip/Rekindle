use super::*;

const fn rect(x: i32, y: i32, w: u32, h: u32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn an_empty_silhouette_is_refused() {
    assert!(Silhouette::new(&[]).is_err());
    assert!(Silhouette::new(&[rect(0, 0, 0, 10), rect(4, 4, 10, 0)]).is_err());
    assert!(Silhouette::full(0, 480).is_err());
}

#[test]
fn a_silhouette_over_the_cap_is_refused() {
    let at_cap = vec![rect(0, 0, 1, 1); MAX_RECTS];
    assert!(Silhouette::new(&at_cap).is_ok());
    let over = vec![rect(0, 0, 1, 1); MAX_RECTS + 1];
    assert!(Silhouette::new(&over).is_err());
}

#[test]
fn empty_rects_are_dropped() {
    let s = Silhouette::new(&[rect(0, 0, 0, 5), rect(1, 2, 3, 4)]).expect("valid");
    assert_eq!(s.rects(), &[rect(1, 2, 3, 4)]);
}

#[test]
fn hit_test_uses_exclusive_edges() {
    // A notched top edge: a 10 px band from x 20 and the body below it.
    let s = Silhouette::new(&[rect(20, 0, 10, 10), rect(0, 10, 50, 40)]).expect("valid");
    assert!(s.contains(20.0, 0.0));
    assert!(s.contains(29.9, 9.9));
    assert!(!s.contains(30.0, 5.0), "right edge is exclusive");
    assert!(!s.contains(5.0, 5.0), "the cut-away corner passes through");
    assert!(s.contains(0.0, 10.0));
    assert!(!s.contains(50.0, 20.0));
    assert!(!s.contains(10.0, 50.0));
    assert!(!s.contains(-0.5, 20.0));
}

#[test]
fn full_covers_the_whole_window() {
    let s = Silhouette::full(320, 650).expect("valid");
    assert!(s.contains(0.0, 0.0));
    assert!(s.contains(319.0, 649.0));
    assert!(!s.contains(320.0, 0.0));
}

#[test]
fn logical_rects_round_outward_at_scale_two() {
    let s = Silhouette::new(&[rect(3, 5, 7, 2), rect(0, 0, 640, 1300)]).expect("valid");
    // 3..10 × 5..7 device → 1..5 × 2..4 logical (floor start, ceil end).
    assert_eq!(to_logical(&s, 2), vec![(1, 2, 4, 2), (0, 0, 320, 650)]);
    assert_eq!(to_logical(&s, 1), vec![(3, 5, 7, 2), (0, 0, 640, 1300)]);
}

#[test]
fn logical_rects_clamp_a_bad_scale_to_one() {
    let s = Silhouette::new(&[rect(-3, 4, 6, 6)]).expect("valid");
    assert_eq!(to_logical(&s, 0), vec![(-3, 4, 6, 6)]);
    // Negative origins floor away from zero: -3 / 2 → -2, 3 / 2 → 2 (ceil).
    assert_eq!(to_logical(&s, 2), vec![(-2, 2, 4, 3)]);
}

#[test]
fn win32_corners_keep_silhouette_order_and_exclusive_ends() {
    let s = Silhouette::new(&[rect(20, 0, 10, 10), rect(0, 0, 0, 3), rect(0, 10, 50, 40)])
        .expect("valid");
    // The first entry is CreateRectRgn's base, the rest are OR'd in order;
    // the empty rectangle never reaches GDI.
    assert_eq!(win32_corners(&s), vec![(20, 0, 30, 10), (0, 10, 50, 50)]);
}

#[test]
fn local_point_on_one_scale() {
    // Window at (200, 100) device px, cursor at (230, 140): local (30, 40).
    let p = local_point((230.0, 140.0), 2.0, (200, 100), 2.0);
    assert!((p.0 - 30.0).abs() < 1e-9 && (p.1 - 40.0).abs() < 1e-9);
}

#[test]
fn local_point_across_scales() {
    // Primary monitor at 1×, window on a 2× display at logical (1000, 50):
    // the cursor at logical (1010, 60) is 10 points = 20 device px inside.
    let p = local_point((1010.0, 60.0), 1.0, (2000, 100), 2.0);
    assert!((p.0 - 20.0).abs() < 1e-9 && (p.1 - 20.0).abs() < 1e-9);
    let s = Silhouette::new(&[rect(0, 0, 16, 16), rect(16, 16, 16, 16)]).expect("valid");
    assert!(s.contains(p.0, p.1));
    assert!(!s.contains(20.0, 10.0));
}
