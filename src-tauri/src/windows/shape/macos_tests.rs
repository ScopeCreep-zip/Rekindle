use super::super::geometry::ShapeRect;
use super::*;

#[test]
fn ignores_only_over_holes_inside_the_window() {
    // A rounded-corner stand-in: the top-left 8×8 is cut away.
    let s = Silhouette::new(&[
        ShapeRect {
            x: 8,
            y: 0,
            w: 92,
            h: 8,
        },
        ShapeRect {
            x: 0,
            y: 8,
            w: 100,
            h: 92,
        },
    ])
    .expect("valid");
    assert!(should_ignore(&s, (2.0, 2.0), (100, 100)), "corner hole");
    assert!(!should_ignore(&s, (50.0, 2.0), (100, 100)), "top band art");
    assert!(!should_ignore(&s, (2.0, 50.0), (100, 100)), "body");
    assert!(
        !should_ignore(&s, (-5.0, 2.0), (100, 100)),
        "outside the window"
    );
    assert!(
        !should_ignore(&s, (100.0, 50.0), (100, 100)),
        "past the right edge"
    );
}
