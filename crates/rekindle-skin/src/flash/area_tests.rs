use super::*;

fn square(side: usize, lit: usize) -> Vec<bool> {
    (0..side * side)
        .map(|i| i % side < lit && i / side < lit)
        .collect()
}

const TILE: DrawnAs = DrawnAs {
    x: AxisFit::Tile,
    y: AxisFit::Tile,
};

#[test]
fn the_field_and_limit_are_note_1s() {
    assert_eq!(MAX_AREA, 21_824);
}

#[test]
fn a_tiled_image_counts_every_repeat_in_the_field() {
    // A 2048² image, its top-left 147² flashing: 21,609 px, under.
    let side = 2048;
    assert!(!exceeds(side, side, &square(side, 147), TILE));
    // 148² = 21,904: over.
    assert!(exceeds(side, side, &square(side, 148), TILE));
    // One pixel of a 16² tile repeats 22 × 16 times in a field.
    let mut one = vec![false; 256];
    one[0] = true;
    assert!(!exceeds(16, 16, &one, TILE));
    // A whole 16² tile flashing fills the field.
    assert!(exceeds(16, 16, &[true; 256], TILE));
}

#[test]
fn a_stretched_pixel_fills_the_field() {
    let mut one = vec![false; 64];
    one[9] = true;
    let stretch = DrawnAs {
        x: AxisFit::Stretch,
        y: AxisFit::Tile,
    };
    assert!(exceeds(8, 8, &one, stretch));
}

#[test]
fn nine_slice_insets_are_one_to_one_and_the_middle_is_not() {
    let slice = DrawnAs {
        x: AxisFit::Slice { start: 4, end: 4 },
        y: AxisFit::Slice { start: 4, end: 4 },
    };
    // A corner pixel: 1 px rendered.
    let mut corner = vec![false; 144];
    corner[0] = true;
    assert!(!exceeds(12, 12, &corner, slice));
    // An edge pixel stretches along its edge.
    let mut edge = vec![false; 144];
    edge[6] = true;
    assert!(exceeds(12, 12, &edge, slice));
}
