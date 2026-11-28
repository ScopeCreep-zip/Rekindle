use super::*;
use crate::registry::ENTRIES;

#[test]
fn slots_name_registry_colours() {
    assert!(slot_tokens_are_colours(ENTRIES));
}

#[test]
fn axes_sample_as_drawn() {
    assert_eq!(sampled(Axis::Stretch, 2, 4), vec![1, 3]);
    assert_eq!(sampled(Axis::Stretch, 8, 4), vec![0, 1, 2, 3]);
    // Centred tiles: a 3 px box over 10 px shows the middle.
    assert_eq!(sampled(Axis::Tile, 3, 10), vec![4, 5, 6]);
    assert_eq!(sampled(Axis::Tile, 12, 10).len(), 10);
    // Insets 2 + 2 of 10, a 20 px box: every pixel.
    assert_eq!(sampled(Axis::Slice(2, 2), 20, 10).len(), 10);
    // A 2 px box: the insets shrink to 1 + 1, the middle is not drawn.
    assert_eq!(sampled(Axis::Slice(2, 2), 2, 10), vec![1, 9]);
}

#[test]
fn the_backed_band_skips_the_outer_slice_pixels() {
    // Insets 3 + 3 of 10 in a 20 px box, backed from 2 px in: source
    // 0, 1, 8 and 9 (the outer edge pixels) are not under the backing.
    assert_eq!(
        backed(Axis::Slice(3, 3), 20, 10, 2, 2),
        (2..8).collect::<Vec<_>>()
    );
    // Backed from the slice in: the centre only.
    assert_eq!(
        backed(Axis::Slice(3, 3), 20, 10, 3, 3),
        (3..7).collect::<Vec<_>>()
    );
    assert_eq!(backed(Axis::Stretch, 2, 4, 1, 1), vec![1, 3]);
}
