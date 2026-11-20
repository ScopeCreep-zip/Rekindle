//! The shaper's reading of the Host's `SkinFrames` (plan SK0 S3): the
//! decoded masks and the layout give the skin's silhouette at the window's
//! device size, and a kind the skin does not frame keeps its rectangle.

use rekindle_types::dto::skin::{FrameTile, Insets, SkinWindow, TileAxis, WindowLayout};
use rekindle_types::dto::skin_library::{SkinFrames, SkinTileMask, SkinWindowFrame};
use tauri::PhysicalSize;

use super::{compute, decode, frame_of};
use crate::window_labels::WindowKind;
use crate::windows::shape::{ShapeRect, Silhouette};

const NO_INSETS: Insets = Insets {
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
};

/// A main frame of one 4×1 title tile across the top whose middle two
/// pixels are opaque, over a content rectangle from the second row down.
fn frames(bits: Vec<u8>) -> SkinFrames {
    SkinFrames {
        id: "builtin:test".to_owned(),
        windows: vec![SkinWindowFrame {
            window: SkinWindow::Main,
            layout: WindowLayout {
                outsets: NO_INSETS,
                content: Insets {
                    top: 1,
                    ..NO_INSETS
                },
                min_width: 4,
                min_height: 1,
                tiles: vec![FrameTile {
                    image: "title.png".to_owned(),
                    width: 4,
                    height: 1,
                    x: TileAxis::Start { offset: 0 },
                    y: TileAxis::Start { offset: 0 },
                    overhang: false,
                    decoration: None,
                }],
            },
            masks: vec![SkinTileMask {
                width: 4,
                height: 1,
                bits,
            }],
        }],
    }
}

fn r(x: i32, y: i32, w: u32, h: u32) -> ShapeRect {
    ShapeRect { x, y, w, h }
}

#[test]
fn the_buddy_list_is_cut_to_the_main_frame_at_its_device_size() {
    let decoded = decode(frames(vec![0b0110_0000])).unwrap();
    let frame = frame_of(WindowKind::BuddyList);
    let at_one = compute(&decoded, frame, PhysicalSize::new(4, 4), 1.0).unwrap();
    assert_eq!(
        at_one,
        Silhouette::new(&[r(1, 0, 2, 1), r(0, 1, 4, 3)]).unwrap()
    );
    let at_two = compute(&decoded, frame, PhysicalSize::new(8, 8), 2.0).unwrap();
    assert_eq!(
        at_two,
        Silhouette::new(&[r(2, 0, 4, 2), r(0, 2, 8, 6)]).unwrap()
    );
}

#[test]
fn a_window_the_skin_does_not_frame_keeps_its_rectangle() {
    let decoded = decode(frames(vec![0b0110_0000])).unwrap();
    let whole = Silhouette::full(4, 4).unwrap();
    // The skin has no chat layout; a login window wears no frame.
    for kind in [WindowKind::Chat, WindowKind::Login] {
        let got = compute(&decoded, frame_of(kind), PhysicalSize::new(4, 4), 1.0).unwrap();
        assert_eq!(got, whole, "{kind:?}");
    }
    // No skin frames at all (the Host refused or the skin has none).
    let none = compute(
        &super::SkinFrameMap::new(),
        frame_of(WindowKind::BuddyList),
        PhysicalSize::new(4, 4),
        1.0,
    )
    .unwrap();
    assert_eq!(none, whole);
}

#[test]
fn a_mask_whose_bits_do_not_match_its_size_refuses_the_answer() {
    assert!(decode(frames(vec![0b0110_0000, 0])).is_err());
    assert!(decode(frames(Vec::new())).is_err());
}
