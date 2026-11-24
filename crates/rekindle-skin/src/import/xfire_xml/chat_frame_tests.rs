//! The chat frame fitted to the pack's chat content region, on a layout
//! shaped like Onyx's and on every built-in's served chat window.

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{FrameTile, SkinWindow, TileAxis, WindowLayout};

use super::super::super::tree::parse;
use super::*;

/// Onyx's `chatwindow.xml` placements (corners 83×87, the chat area 8 px
/// in and 67 down, the edit box 8 px above the bottom).
const ONYX: &[u8] = br#"<Tile Name="OnyxChatWindow" Component="ChatWindow">
<Tile Name="Middle" X="83" Y="87" IndentRight="83" IndentBottom="87" ResizeX="100" ResizeY="100" Component="CMiddle"/>
<Tile Name="ChatArea" Z="5" X="8" IndentRight="8" Y="67" IndentBottom="75" ResizeX="100" ResizeY="100" Component="ChatArea"/>
<Tile Name="ChatEdit" Z="45" X="8" IndentRight="57" Y="ChatArea.BOTTOM+8" ResizeX="100" ResizeY="100" IndentBottom="8" Component="ChatEdit"/>
</Tile>"#;

/// Onyx's `components.xml` `ChatWindow` (`MinWidth="253" MinHeight="280"`).
const ONYX_COMPONENTS: &[u8] =
    br#"<Components><Component Name="ChatWindow" Class="MainWindow" MinWidth="253" MinHeight="280"/></Components>"#;

fn solid(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba([20, 20, 20, 255]))
}

/// The art cut at its middle, as Onyx's is: 83 wide sides, 87 tall rows.
fn halves() -> [RgbaImage; 9] {
    std::array::from_fn(|i| {
        solid(
            if i % 3 == 1 { 84 } else { 83 },
            if i / 3 == 1 { 86 } else { 87 },
        )
    })
}

#[test]
fn a_frame_cut_at_its_middle_serves_the_packs_chat_region() {
    let layout = parse("chatwindow.xml", ONYX).unwrap();
    let components = [parse("components.xml", ONYX_COMPONENTS).unwrap()];
    let mut pieces = halves();
    let fitted = fit(&layout, &components, &mut pieces);
    assert_eq!(fitted.region, Some([8, 67, 8, 8]));
    assert_eq!(fitted.declared, Some((253, 280)));
    let (built, _) = window(&pieces, &fitted).unwrap();
    let c = built.content;
    assert_eq!((c.top, c.right, c.bottom, c.left), (67, 8, 8, 8));
    assert_eq!((built.min_width, built.min_height), (253, 280));
}

/// Without a declared minimum the window holds its insets and one copy of
/// the centre fill, never only its trimmed pieces.
#[test]
fn an_undeclared_minimum_holds_the_content_region() {
    let layout = parse("chatwindow.xml", ONYX).unwrap();
    let mut pieces = halves();
    let fitted = fit(&layout, &[], &mut pieces);
    assert_eq!(fitted.declared, None);
    let (built, _) = window(&pieces, &fitted).unwrap();
    assert_eq!(
        (built.min_width, built.min_height),
        // The width is the corners' (83 + 83), wider than the content's need.
        (83 + 83, 67 + 8 + 86)
    );
}

/// Space Cowboy's edge pieces: a 2 px line inside a transparent margin.
#[test]
fn a_transparent_outer_margin_is_cut_from_its_side() {
    let line = |w: u32, h: u32, keep: fn(u32, u32, u32, u32) -> bool| {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([200, 0, 0, if keep(x, y, w, h) { 255 } else { 0 }])
        })
    };
    let mut pieces: [RgbaImage; 9] = std::array::from_fn(|i| match i {
        0..=2 => line(20, 31, |_, y, _, _| y >= 29),
        6..=8 => line(20, 57, |_, y, _, _| y < 2),
        3 => line(20, 50, |x, _, _, _| x >= 18),
        5 => line(24, 50, |x, _, _, _| x < 2),
        _ => solid(4, 4),
    });
    // The corners keep their outer margin only where the rows' and sides'
    // margins meet; the common margin is what is cut.
    assert_eq!(trim(&mut pieces), [0, 29, 0, 55]);
    assert!(pieces[..3].iter().all(|p| p.height() == 2));
    assert!(pieces[6..].iter().all(|p| p.height() == 2));
}

/// Combat's top row: the middle piece clear on rows 0–1, the corners with
/// knobs there. The top edge is cut to the middle's art, and the knobs are
/// laid whole above the frame as overhang, flush with the window's ends.
#[test]
fn corner_knobs_above_a_clear_top_middle_are_overhang_art() {
    let paint = |w: u32, h: u32, clear_rows: u32, knob: u32| {
        RgbaImage::from_fn(w, h, |x, y| {
            let art = y >= clear_rows || x < knob;
            Rgba([90, 90, 90, if art { 255 } else { 0 }])
        })
    };
    let mut pieces: [RgbaImage; 9] = std::array::from_fn(|i| match i {
        0 => paint(10, 12, 2, 3),
        1 => paint(4, 12, 2, 0),
        2 => paint(10, 12, 2, 7),
        _ => solid(10, 10),
    });
    let layout = parse("chatwindow.xml", ONYX).unwrap();
    let fitted = fit(&layout, &[], &mut pieces);
    assert!(
        pieces[..3].iter().all(|p| p.height() == 10),
        "top row cut by 2"
    );
    // The region's top is counted from the cut edge: 67 − 2.
    assert_eq!(fitted.region.map(|r| r[1]), Some(65));
    assert!(fitted.crest[1].is_none(), "no bottom crest");
    let knobs = fitted.crest[0].clone().expect("the knobs are kept");
    assert_eq!(
        knobs.each_ref().map(|c| (c.width(), c.height())),
        [(10, 2); 2]
    );
    let (built, files) = window(&pieces, &fitted).unwrap();
    assert_eq!(built.outsets.top, 2);
    let over: Vec<_> = built.tiles.iter().filter(|t| t.overhang).collect();
    let ends: Vec<_> = over.iter().map(|t| (t.x, t.y, t.height)).collect();
    let top = TileAxis::Start { offset: 0 };
    assert_eq!(
        ends,
        [
            (TileAxis::Start { offset: 0 }, top, 2),
            (TileAxis::End { offset: 0 }, top, 2)
        ]
    );
    assert!(over.iter().all(|t| files.contains_key(&t.image)));
    // A top row whose middle is as clear as its corners keeps no crest.
    let mut plain = halves();
    assert!(crest(&mut plain, true).is_none());
    // The bottom row mirrored keeps its knobs below the frame.
    let mut flipped: [RgbaImage; 9] = std::array::from_fn(|i| match i {
        6 => flip(&paint(10, 12, 2, 3)),
        7 => flip(&paint(4, 12, 2, 0)),
        8 => flip(&paint(10, 12, 2, 7)),
        _ => solid(10, 10),
    });
    let (feet, rows) = crest(&mut flipped, false).expect("the bottom knobs are kept");
    assert_eq!((feet[0].height(), rows, flipped[7].height()), (2, 2, 10));
    assert!(feet[0].get_pixel(0, 1)[3] > 0 && feet[0].get_pixel(5, 1)[3] == 0);
}

fn flip(img: &RgbaImage) -> RgbaImage {
    let h = img.height();
    RgbaImage::from_fn(img.width(), h, |x, y| *img.get_pixel(x, h - 1 - y))
}

/// The extent [lo, hi) of a tile along an axis of `len` px.
fn extent(axis: TileAxis, size: u16, len: u32) -> (u32, u32) {
    let size = u32::from(size);
    match axis {
        TileAxis::Start { offset } => (u32::from(offset), u32::from(offset) + size),
        TileAxis::End { offset } => (len - u32::from(offset) - size, len - u32::from(offset)),
        TileAxis::Centre => ((len - size) / 2, (len - size) / 2 + size),
        TileAxis::Repeat { start, end } => (u32::from(start), len - u32::from(end)),
    }
}

/// A window edge, in the order the test walks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

/// The sides of each built-in's chat window that have no frame piece by
/// the pack's design, with the reason; every other side's pieces must
/// reach its window edge. Exact: a pack gaining or losing a side fails.
const EDGELESS: &[(&str, &[Side], &str)] = &[(
    "xfire-classic",
    &[Side::Top, Side::Bottom],
    "v3 frame has no top or bottom row (xfirecomponents.xml: the outer \
     edges' middles, 9 px rails)",
)];

/// Whether the frame pieces laid flush on the window's `side`, each with
/// art on its outer line, cover that whole edge at `w`×`h`; `None` when
/// the side has no such piece.
fn edge_covered(
    layout: &WindowLayout,
    files: &crate::package::Package<'_>,
    (w, h): (u32, u32),
    side: Side,
) -> Option<bool> {
    let flush = |t: &FrameTile| match side {
        Side::Top => t.y == TileAxis::Start { offset: 0 },
        Side::Right => t.x == TileAxis::End { offset: 0 },
        Side::Bottom => t.y == TileAxis::End { offset: 0 },
        Side::Left => t.x == TileAxis::Start { offset: 0 },
    };
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for t in layout
        .tiles
        .iter()
        .filter(|t| t.decoration.is_none() && !t.overhang && flush(t))
    {
        let img = crate::images::decode_png(files.get(&t.image).unwrap()).unwrap();
        let (iw, ih) = (img.width(), img.height());
        let across = matches!(side, Side::Top | Side::Bottom);
        let art = match side {
            Side::Top => (0..iw).any(|x| img.get_pixel(x, 0)[3] > 0),
            Side::Right => (0..ih).any(|y| img.get_pixel(iw - 1, y)[3] > 0),
            Side::Bottom => (0..iw).any(|x| img.get_pixel(x, ih - 1)[3] > 0),
            Side::Left => (0..ih).any(|y| img.get_pixel(0, y)[3] > 0),
        };
        if art {
            spans.push(if across {
                extent(t.x, t.width, w)
            } else {
                extent(t.y, t.height, h)
            });
        }
    }
    if spans.is_empty() {
        return None;
    }
    spans.sort_unstable();
    let len = if matches!(side, Side::Top | Side::Bottom) {
        w
    } else {
        h
    };
    let mut reached = 0;
    for (lo, hi) in spans {
        if lo > reached {
            return Some(false);
        }
        reached = reached.max(hi);
    }
    Some(reached >= len)
}

/// The chat window's content insets each built-in serves, before this fix
/// cut at the art's middle (Onyx 87/83/87/83, xSLIMFIRE 165/125/165/125)
/// or left Space Cowboy's transparent margins inside the window. Each
/// side with an edge piece reaches its window edge at the minimum and at
/// 1200×900; the sides without one are exactly [`EDGELESS`]'s.
#[test]
fn every_builtin_chat_frame_fits_its_border_and_reaches_the_window_edges() {
    let mut seen = Vec::new();
    for pack in &crate::builtin::PACKS {
        let files = crate::package::read(pack.bytes).unwrap();
        let m = crate::manifest::parse(&files).unwrap();
        let Some(chat) = m.windows.get(&SkinWindow::Chat) else {
            continue;
        };
        let c = chat.content;
        let (mw, mh) = (u32::from(chat.min_width), u32::from(chat.min_height));
        let id = pack.id;
        assert!(
            2 * u32::from(c.left) < mw && 2 * u32::from(c.right) < mw,
            "builtin:{id} chat sides {c:?} at min width {mw}"
        );
        assert!(
            2 * u32::from(c.bottom) < mh && u32::from(c.top) + u32::from(c.bottom) < mh,
            "builtin:{id} chat rows {c:?} at min height {mh}"
        );
        let edgeless = EDGELESS
            .iter()
            .find(|(e, _, _)| *e == id)
            .map_or(&[][..], |(_, sides, _)| *sides);
        seen.push(id);
        for size in [(mw, mh), (1200, 900)] {
            for side in SIDES {
                match edge_covered(chat, &files, size, side) {
                    None => assert!(
                        edgeless.contains(&side),
                        "builtin:{id} chat {side:?} has no edge piece at {size:?}, not named in EDGELESS"
                    ),
                    Some(covered) => {
                        assert!(
                            !edgeless.contains(&side),
                            "builtin:{id} chat {side:?} has an edge piece, but EDGELESS names it"
                        );
                        assert!(covered, "builtin:{id} chat {side:?} edge uncovered at {size:?}");
                    }
                }
            }
        }
    }
    for (id, _, why) in EDGELESS {
        assert!(
            seen.contains(id),
            "EDGELESS names builtin:{id} ({why}), no chat window"
        );
    }
    let content = |id: &str| {
        let files = crate::package::read(crate::builtin::find(id).unwrap().1.bytes).unwrap();
        let c = crate::manifest::parse(&files).unwrap().windows[&SkinWindow::Chat].content;
        (c.top, c.right, c.bottom, c.left)
    };
    assert_eq!(content("builtin:onyx"), (67, 8, 8, 8));
    assert_eq!(content("builtin:xslimfire"), (54, 18, 37, 18));
    assert_eq!(content("builtin:spacecowboy"), (53, 9, 11, 9));
    assert_eq!(content("builtin:symbiosis"), (13, 13, 13, 13));
}
