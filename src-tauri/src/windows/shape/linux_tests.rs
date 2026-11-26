use super::super::geometry::ShapeRect;
use super::*;

#[test]
fn region_keeps_the_silhouette_and_drops_the_cut_away_corner() {
    let s = Silhouette::new(&[
        ShapeRect {
            x: 20,
            y: 0,
            w: 10,
            h: 10,
        },
        ShapeRect {
            x: 0,
            y: 10,
            w: 50,
            h: 40,
        },
    ])
    .expect("valid");
    let r = region(&s, 1);
    assert!(r.contains_point(25, 5));
    assert!(r.contains_point(0, 10));
    assert!(!r.contains_point(5, 5));
    assert!(!r.contains_point(50, 20));
    let r2 = region(&s, 2);
    assert!(r2.contains_point(10, 0));
    assert!(!r2.contains_point(25, 20));
}
