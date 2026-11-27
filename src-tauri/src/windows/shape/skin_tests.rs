//! The shaper's reading of the Host's `SkinFrames` (plan SK0 S3): the
//! decoded masks and the layout give the skin's silhouette at the window's
//! device size, and a kind the skin does not frame keeps its rectangle.

use rekindle_skin::shape::ALPHA_OPAQUE;
use rekindle_types::dto::skin::{FrameTile, Insets, SkinWindow, TileAxis, WindowLayout};
use rekindle_types::dto::skin_library::{SkinFrames, SkinTileMask, SkinWindowFrame};
use tauri::PhysicalSize;

use super::{compute, decode, frame_of, frame_of_label, kind_of, min_window};
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
        tokens_href: "skin://test/tokens.css".to_owned(),
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
                size_hints: Default::default(),
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
                faint: Vec::new(),
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
fn fractional_scale_preserves_art_edges_and_reaches_the_native_client_edge() {
    let decoded = decode(frames(vec![0b0110_0000])).unwrap();
    for (scale, title_width) in [(1.25, 2), (1.5, 3), (1.75, 4)] {
        let shape = compute(
            &decoded,
            Some(SkinWindow::Main),
            PhysicalSize::new(8, 8),
            scale,
        )
        .unwrap();
        assert_eq!(
            shape,
            Silhouette::new(&[r(1, 0, title_width, 1), r(0, 1, 8, 7)]).unwrap(),
            "{scale}×"
        );
        assert!(shape.contains(7.5, 7.5));
        assert!(!shape.contains(8.0, 7.5));
        assert!(!shape.contains(0.5, 0.5));
    }
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
    assert_eq!(
        frame_of_label("community-0123456789abcdef"),
        Some(SkinWindow::Community)
    );
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

/// Plan SK0 over E7.7 (TAB1 reverted): Symbiosis's InfoView tab
/// (`MainInfoViewButton`, 13×115, `JustX="Right"` in the frame's
/// `IndentRight="13"` column) protrudes beside the main frame in a 13 px
/// right outset. The tab is shaped art (SK0: the art is the window's
/// shape, `rekindle_skin::shape::outline_nodes` puts it in the outline), so
/// the silhouette built from the served layout
/// (`rekindle_skin::shape::silhouette`, through [`compute`]) holds every
/// pixel of the tab's normal art at or above the shaper's threshold
/// ([`ALPHA_OPAQUE`], `rekindle_skin::shape::mask`) and leaves every
/// transparent one in its box out, as it leaves the outset column above
/// and below the tab out, so those pixels are click-through; the frame's
/// own right edge beside the tab stays opaque.
#[test]
fn symbiosis_info_view_tab_protrudes_and_the_column_around_it_is_click_through() {
    let (_, pack) = rekindle_skin::builtin::find("builtin:symbiosis").unwrap();
    let canonical = rekindle_skin::validate::run(pack.bytes).unwrap();
    let windows = rekindle_skin::serve::frames(&canonical.bytes, &canonical.address()).unwrap();
    let decoded = decode(SkinFrames {
        id: "builtin:symbiosis".to_owned(),
        tokens_href: rekindle_skin::serve::tokens_href(&canonical.address()),
        windows,
    })
    .unwrap();
    let (layout, _) = &decoded[&SkinWindow::Main];
    assert_eq!(layout.outsets.right, 13, "{:?}", layout.outsets);
    // The reference screenshot's main window (`shape/tests.rs` goldens).
    let (w, h) = (399_i32, 555_i32);
    let tree = layout.tree.as_ref().expect("a tree-laid main window");
    let (tab, [x, y, bw, bh]) = tree
        .boxes(w, h)
        .into_iter()
        .find(|(n, _)| n.id == "MainInfoViewButton")
        .expect("the InfoView tab");
    let column = w - i32::from(layout.outsets.right);
    assert_eq!((bw, bh), (13, 115));
    assert!(
        x >= column && x + bw <= w,
        "tab {x}+{bw} outside the outset column {column}..{w}"
    );
    let shape = compute(
        &decoded,
        frame_of(WindowKind::BuddyList),
        PhysicalSize::new(w.unsigned_abs(), h.unsigned_abs()),
        1.0,
    )
    .unwrap();
    let inside = |px: i32, py: i32| shape.contains(f64::from(px) + 0.5, f64::from(py) + 0.5);
    // The tab's normal art, the one image the outline lays at its box's
    // top left (`rekindle_skin::shape::of_layout`).
    let art = tab.art.as_ref().expect("the tab's art");
    assert_eq!(art.normal.len(), 1, "{:?}", art.normal);
    let piece = art.normal.values().next().expect("the tab's normal piece");
    assert_eq!((piece.width, piece.height), (13, 115));
    let package = rekindle_skin::package::read(&canonical.bytes).unwrap();
    let png = package.get(&piece.image).expect("the tab's image");
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .unwrap()
        .into_rgba8();
    assert_eq!(img.dimensions(), (13, 115));
    let (mut opaque, mut clear) = (0_u32, 0_u32);
    for (ax, ay, pixel) in img.enumerate_pixels() {
        let px = x + i32::try_from(ax).unwrap();
        let py = y + i32::try_from(ay).unwrap();
        if pixel.0[3] >= ALPHA_OPAQUE {
            opaque += 1;
            assert!(inside(px, py), "opaque tab pixel ({px}, {py}) cut out");
        } else {
            clear += 1;
            assert!(
                !inside(px, py),
                "transparent tab pixel ({px}, {py}) is not click-through"
            );
        }
    }
    assert!(
        opaque > 0 && clear > 0,
        "shaped tab art: {opaque} opaque, {clear} clear"
    );
    for px in column..w {
        for py in (0..y).chain(y + bh..h) {
            assert!(
                !inside(px, py),
                "outset pixel ({px}, {py}) is not click-through"
            );
        }
    }
    assert!(
        inside(column - 1, y + bh / 2),
        "the frame's right edge beside the tab"
    );
}
