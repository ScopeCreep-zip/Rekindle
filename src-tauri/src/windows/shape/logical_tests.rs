use super::*;
use crate::windows::shape::geometry::ShapeRect;

fn rect(x: i32, y: i32, w: u32, h: u32) -> ShapeRect {
    ShapeRect { x, y, w, h }
}

#[test]
fn canonical_union_rounds_out_clips_and_stays_surface_local() {
    let silhouette = Silhouette::new(&[
        rect(5, 3, 14, 6),
        rect(8, 1, 18, 8),
        rect(5, 3, 14, 6),
        rect(-8, 14, 11, 10),
    ])
    .unwrap();
    let shape = LogicalShape::project(&silhouette, (12, 10), 2).unwrap();
    assert_eq!(shape.rects, vec![(4, 0, 8, 1), (2, 1, 10, 4), (0, 7, 2, 3)]);
    assert_eq!(shape.bounds, (0, 0, 12, 10));
    assert_eq!(shape.margins, Margins::default());
    assert!(LogicalShape::project(&silhouette, (0, 10), 2).is_none());
}

#[test]
fn the_bounds_keep_four_insets_without_filling_an_internal_hole() {
    let silhouette = Silhouette::new(&[
        rect(6, 4, 8, 1),
        rect(6, 5, 1, 3),
        rect(13, 5, 1, 3),
        rect(6, 8, 8, 1),
    ])
    .unwrap();
    let shape = LogicalShape::project(&silhouette, (22, 18), 1).unwrap();
    assert_eq!(shape.bounds, (6, 4, 8, 5));
    assert_eq!(
        shape.margins,
        Margins {
            left: 6,
            top: 4,
            right: 8,
            bottom: 9
        }
    );
    assert_eq!(
        shape.rects,
        vec![(6, 4, 8, 1), (6, 5, 1, 3), (13, 5, 1, 3), (6, 8, 8, 1)]
    );
    assert_eq!(
        shape.margins.canvas(shape.margins.visible((22, 18))),
        (22, 18)
    );
    let outside = Silhouette::new(&[rect(22, 0, 1, 1)]).unwrap();
    assert!(LogicalShape::project(&outside, (22, 18), 1).is_none());
}

#[test]
fn an_equal_size_pack_change_can_replace_the_region_without_moving_its_bounds() {
    let filled = Silhouette::new(&[rect(4, 6, 8, 4)]).unwrap();
    let open = Silhouette::new(&[rect(4, 6, 2, 4), rect(10, 6, 2, 4)]).unwrap();
    let canvas = (18, 14);
    let a = LogicalShape::project(&filled, canvas, 1).unwrap();
    let b = LogicalShape::project(&open, canvas, 1).unwrap();
    assert_eq!(a.bounds, b.bounds);
    assert_eq!(a.margins, b.margins);
    assert_ne!(a.rects, b.rects);
}
