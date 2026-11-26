//! The shaper's reading of the Host's `SkinFrames` (plan SK0 S3): the
//! decoded masks and the layout give the skin's silhouette at the window's
//! device size, and a kind the skin does not frame keeps its rectangle.

use rekindle_types::dto::skin::{FrameTile, Insets, SkinWindow, TileAxis, WindowLayout};
use rekindle_types::dto::skin_library::{SkinFrames, SkinTileMask, SkinWindowFrame};
use tauri::PhysicalSize;

use super::{compute, decode, frame_of, frame_of_label, grown, kind_of, min_window};
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
                    fill: false,
                }],
                tree: None,
                title: None,
                popups: std::collections::BTreeMap::new(),
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

/// Plan SKW1: a community window with a community wears the community frame
/// (composed from the pack's chat window); the community browser shares the
/// Community kind but keeps its rectangle; the buddy list and a chat window
/// keep theirs.
#[test]
fn a_community_window_wears_the_community_frame_and_the_browser_none() {
    assert_eq!(frame_of(WindowKind::Community), Some(SkinWindow::Community));
    assert_eq!(frame_of_label("community-0123456789abcdef"), Some(SkinWindow::Community));
    assert_eq!(frame_of_label("community-browser"), None);
    assert_eq!(frame_of_label("buddy-list"), Some(SkinWindow::Main));
    assert_eq!(frame_of_label("settings"), None);
}

#[test]
fn a_mask_whose_bits_do_not_match_its_size_refuses_the_answer() {
    assert!(decode(frames(vec![0b0110_0000, 0])).is_err());
    assert!(decode(frames(Vec::new())).is_err());
}

/// The window a frame needs is the frame's served minimum grown by its
/// outsets (plan SK2-N4); a kind the skin does not frame has none.
#[test]
fn a_framed_window_needs_the_frames_minimum_plus_its_outsets() {
    let mut served = frames(vec![0b0110_0000]);
    served.windows[0].layout.min_width = 300;
    served.windows[0].layout.min_height = 500;
    served.windows[0].layout.outsets = Insets {
        top: 2,
        right: 6,
        bottom: 4,
        left: 8,
    };
    let decoded = decode(served).unwrap();
    assert_eq!(
        min_window(&decoded, frame_of(WindowKind::BuddyList)),
        Some((314.0, 506.0))
    );
    // The skin lays no chat frame; a login window wears none.
    assert_eq!(min_window(&decoded, frame_of(WindowKind::Chat)), None);
    assert_eq!(min_window(&decoded, frame_of(WindowKind::Login)), None);
    assert_eq!(kind_of(SkinWindow::Main), WindowKind::BuddyList);
    assert_eq!(kind_of(SkinWindow::Chat), WindowKind::Chat);
}

/// A window smaller than its skin's minimum grows on the short axis only;
/// one that holds it (a user's larger window) is never shrunk.
#[test]
fn a_window_grows_to_its_skin_and_never_shrinks() {
    let min = (348.0, 500.0);
    assert_eq!(grown((320.0, 650.0), min), Some((348.0, 650.0)));
    assert_eq!(grown((320.0, 400.0), min), Some((348.0, 500.0)));
    assert_eq!(grown((348.0, 500.0), min), None);
    assert_eq!(grown((900.0, 900.0), min), None);
}
