use super::super::geometry::ShapeRect;
use super::*;

fn notched() -> Silhouette {
    Silhouette::new(&[
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
    .expect("valid")
}

fn rects_of(region: &Region) -> Vec<(i32, i32, i32, i32)> {
    (0..region.num_rectangles())
        .map(|i| {
            let r = region.rectangle(i);
            (r.x(), r.y(), r.width(), r.height())
        })
        .collect()
}

#[test]
fn region_keeps_the_silhouette_and_drops_the_cut_away_corner() {
    let s = notched();
    let r = region(&s, 1);
    assert!(r.contains_point(25, 5));
    assert!(r.contains_point(0, 10));
    assert!(!r.contains_point(5, 5));
    assert!(!r.contains_point(50, 20));
    let r2 = region(&s, 2);
    assert!(r2.contains_point(10, 0));
    assert!(!r2.contains_point(25, 20));
}

#[test]
fn the_region_handed_to_the_widget_is_exactly_the_silhouette_rects() {
    let s = notched();
    assert_eq!(
        rects_of(&region(&s, 1)),
        vec![(20, 0, 10, 10), (0, 10, 50, 40)]
    );
    assert_eq!(
        rects_of(&region(&s, 2)),
        vec![(10, 0, 5, 5), (0, 5, 25, 20)]
    );
    assert_eq!(rects_of(&region(&s, 2)), geometry::to_logical(&s, 2));
}
