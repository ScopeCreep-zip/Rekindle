use super::*;

/// Whole logical pixels, compared within float precision.
fn px(got: f64, want: f64) {
    assert!((got - want).abs() < f64::EPSILON, "got {got}, want {want}");
}

#[test]
fn the_wanted_size_is_the_default_or_initial_size_within_the_served_limits() {
    // Symbiosis under the 400 ceiling: the initial 320 rises to its minimum.
    px(wanted(320.0, 340, None, Some(400)), 340.0);
    px(wanted(320.0, 400, None, Some(400)), 400.0);
    // A pack default wins over the initial size.
    px(wanted(650.0, 555, Some(555), Some(900)), 555.0);
    // Total Chaos: fixed at its minimum.
    px(wanted(320.0, 470, None, Some(470)), 470.0);
    // A pack maximum lowers a default above it.
    px(wanted(320.0, 300, Some(440), Some(430)), 430.0);
    px(wanted(650.0, 500, None, Some(600)), 600.0);
}

#[test]
fn the_native_limits_are_the_served_ones() {
    let sizing = Sizing::for_frame(SkinWindow::Main, None);
    let served =
        rekindle_skin::shape::max_size(SkinWindow::Main, (300, 500), &WindowSizeHints::default());
    assert_eq!(
        sizing.maximum,
        (served.0.map(f64::from), served.1.map(f64::from))
    );
}
