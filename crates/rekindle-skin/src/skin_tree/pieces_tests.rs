//! The piece-set geometry (spec A7): caps once at natural size, edges and
//! middle tiled from the cap's end edge, every copy clipped to its band.

use rekindle_types::dto::skin_tree::{NodePiece, NodePieces, PieceSlot};

use super::{draw, insets, natural, Copy};

fn set(pieces: &[(PieceSlot, u16, u16)]) -> NodePieces {
    pieces
        .iter()
        .map(|&(s, width, height)| {
            (
                s,
                NodePiece {
                    image: format!("{}.png", s.key()),
                    width,
                    height,
                },
            )
        })
        .collect()
}

fn of(copies: &[Copy], slot: PieceSlot) -> Vec<([i32; 4], (i32, i32))> {
    copies
        .iter()
        .filter(|c| c.slot == slot)
        .map(|c| (c.at, c.src))
        .collect()
}

/// Total Chaos's `MainTitleBar`: 75 px caps, an 8 px middle tiled from
/// the left cap's end edge, the caps never repeated.
#[test]
fn a_row_set_tiles_its_middle_from_the_start_cap() {
    let p = set(&[
        (PieceSlot::Left, 75, 52),
        (PieceSlot::Middle, 8, 52),
        (PieceSlot::Right, 75, 52),
    ]);
    assert_eq!(insets(&p), [75, 75, 0, 0]);
    assert_eq!(natural(&p), (158, 52));
    let d = draw(&p, 170, 52);
    assert_eq!(of(&d, PieceSlot::Left), vec![([0, 0, 75, 52], (0, 0))]);
    assert_eq!(of(&d, PieceSlot::Right), vec![([95, 0, 75, 52], (0, 0))]);
    let mid = of(&d, PieceSlot::Middle);
    assert_eq!(mid.first(), Some(&([75, 0, 8, 52], (0, 0))));
    // 20 px between the caps: two whole copies and a 4 px cut one.
    assert_eq!(mid.last(), Some(&([91, 0, 4, 52], (0, 0))));
    assert_eq!(mid.len(), 3);
}

/// Classic's `MainTitleBar` (`topleft`/`top`/`topright`) and a nine-piece
/// frame: the right cap is laid from the box's right edge, the sides
/// repeat down between the corner rows.
#[test]
fn a_frame_lays_its_corners_and_tiles_its_sides() {
    let p = set(&[
        (PieceSlot::TopLeft, 27, 35),
        (PieceSlot::Top, 43, 35),
        (PieceSlot::TopRight, 27, 35),
        (PieceSlot::Left, 27, 45),
        (PieceSlot::Middle, 42, 58),
        (PieceSlot::Right, 20, 51),
        (PieceSlot::BottomLeft, 27, 35),
        (PieceSlot::Bottom, 43, 35),
        (PieceSlot::BottomRight, 27, 35),
    ]);
    let d = draw(&p, 120, 150);
    assert_eq!(of(&d, PieceSlot::TopRight), vec![([93, 0, 27, 35], (0, 0))]);
    assert_eq!(
        of(&d, PieceSlot::BottomRight),
        vec![([93, 115, 27, 35], (0, 0))]
    );
    // The right side is 20 wide, laid at the box's right edge, from y 35
    // down to 115: 51 px and a 29 px cut copy.
    assert_eq!(
        of(&d, PieceSlot::Right),
        vec![([100, 35, 20, 51], (0, 0)), ([100, 86, 20, 29], (0, 0))]
    );
    // The top edge runs 27..93 (66 px): 43 px and a 23 px cut copy.
    assert_eq!(
        of(&d, PieceSlot::Top),
        vec![([27, 0, 43, 35], (0, 0)), ([70, 0, 23, 35], (0, 0))]
    );
    assert_eq!(of(&d, PieceSlot::Middle)[0], ([27, 35, 42, 58], (0, 0)));
}

/// A box narrower than its caps: the right cap, laid from the right edge,
/// is cut on its left (its source offset), and nothing tiles.
#[test]
fn a_box_smaller_than_its_caps_crops_them() {
    let p = set(&[
        (PieceSlot::Left, 10, 4),
        (PieceSlot::Middle, 3, 4),
        (PieceSlot::Right, 10, 4),
    ]);
    let d = draw(&p, 16, 4);
    assert_eq!(of(&d, PieceSlot::Right), vec![([6, 0, 10, 4], (0, 0))]);
    assert!(of(&d, PieceSlot::Middle).is_empty());
    let d = draw(&p, 8, 4);
    assert_eq!(of(&d, PieceSlot::Left), vec![([0, 0, 8, 4], (0, 0))]);
    assert_eq!(of(&d, PieceSlot::Right), vec![([0, 0, 8, 4], (2, 0))]);
}
