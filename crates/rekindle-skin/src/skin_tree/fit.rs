//! The window sized to the pack's tree (plan SK2 decisions 5 and 14): the
//! overhang, where the frame rows lie, the strips an edge node's art
//! reaches into given up (plan E7.7 TAB1), the smallest window every node fits,
//! and the views node's box as the content rectangle.

use rekindle_types::dto::skin::{Insets, PopupAnchor, PopupEdge, WindowLayout};
use rekindle_types::dto::skin_tree::{Anchor, NodeRole, NodeSpan, SkinNode, SkinTree};

use super::import::bad;
use super::validate::shown;
use crate::import::ImportError;

/// The largest window the fit is searched in, px.
const MOST: i32 = 4096;

/// Whether every node of `tree` laid in a `w × h` window has a box (no
/// negative length) and, when it is shown and has a size on an axis, lies
/// inside.
fn fits(tree: &SkinTree, w: i32, h: i32, x_axis: bool) -> bool {
    tree.boxes(w, h).iter().all(|(n, b)| {
        let (at, len, win) = if x_axis {
            (b[0], b[2], w)
        } else {
            (b[1], b[3], h)
        };
        len >= 0 && (len == 0 || !shown(n) || (at >= 0 && at + len <= win))
    })
}

/// How far the nodes of `tree` that `keep` picks reach outside its own
/// window (laid at the largest size, where only offsets lay a node outside).
fn reach(tree: &SkinTree, keep: impl Fn(&SkinNode) -> bool) -> Insets {
    let boxes: Vec<[i32; 4]> = tree
        .boxes(MOST, MOST)
        .into_iter()
        .filter(|(n, b)| b[2] > 0 && b[3] > 0 && keep(n))
        .map(|(_, b)| b)
        .collect();
    let side = |f: &dyn Fn(&[i32; 4]) -> i32| {
        boxes
            .iter()
            .map(f)
            .max()
            .map_or(0, |v| u16::try_from(v.max(0)).unwrap_or(u16::MAX))
    };
    Insets {
        left: side(&|b| -b[0]),
        top: side(&|b| -b[1]),
        right: side(&|b| b[0] + b[2] - MOST),
        bottom: side(&|b| b[1] + b[3] - MOST),
    }
}

/// The pack's overhang (plan SK2 lead decision 14): how far the nodes that
/// draw art or take input reach outside its window. An art-less wrapper
/// never grows the window (Classic's `StatusWrapper` `ResizeX="100"
/// IndentRight="30"`, a `Group` whose three slices are the title strip's).
fn overhang(tree: &SkinTree) -> Insets {
    reach(tree, shown)
}

/// Where the pack places the frame's own components, `rows` (a row stack's
/// tiles, `stack::layout`, or the tile laying a nine-piece set), past its
/// window's (left, top, right, bottom) edges, negative inside it (StealthX's
/// `MainBackground` `X="112"`, the rail left of it: plan SK2-S1o): the frame rectangle is
/// their box, so the frame art and the tree agree (plan SK2 lead decision
/// 14: Total Chaos's rows `OutdentLeft="1"`, its `BottomBar` 5 px past the
/// box). The stack lays every row across the whole frame rectangle, so the
/// rows must share one horizontal span.
fn frame_reach(tree: &SkinTree, rows: &[String]) -> Result<[i32; 4], ImportError> {
    let spans: Vec<(i32, i32)> = tree
        .boxes(MOST, MOST)
        .into_iter()
        .filter(|(n, _)| rows.contains(&n.id))
        .map(|(_, b)| (b[0], b[2]))
        .collect();
    if spans.len() != rows.len() {
        return Err(bad(format!("frame rows {rows:?} are not all in the tree")));
    }
    if spans.windows(2).any(|p| p[0] != p[1]) {
        return Err(bad(format!(
            "frame rows {rows:?} differ in horizontal span"
        )));
    }
    // An edge inside the window bounds the frame only where the rows follow
    // the window's edge (the same at two sizes); else only what reaches past.
    let at = |m: i32| {
        let b: Vec<[i32; 4]> = tree
            .boxes(m, m)
            .into_iter()
            .filter(|(n, _)| rows.contains(&n.id))
            .map(|(_, b)| b)
            .collect();
        let far = |f: &dyn Fn(&[i32; 4]) -> i32| b.iter().map(f).max().unwrap_or(0);
        [
            far(&|b| -b[0]),
            far(&|b| -b[1]),
            far(&|b| b[0] + b[2] - m),
            far(&|b| b[1] + b[3] - m),
        ]
    };
    let (a, b) = (at(MOST), at(MOST - 100));
    Ok(std::array::from_fn(|i| {
        if a[i] == b[i] {
            a[i]
        } else {
            a[i].max(0)
        }
    }))
}

/// One offset anchored on a side whose strip, `s` px between the frame's
/// edge and the window's, is given up (owner rule, plan E7.7 TAB1):
/// measured from the frame's edge; a node that lay in the strip sits on it.
fn onto_edge(x: i16, s: i16) -> i16 {
    x.saturating_sub(s).max(0)
}

/// `at` re-measured from the frame's edges, `(start, end)` the strips given
/// up on its axis: a centred node keeps the frame the pack centred it in
/// (the window, the strip widening the frame by it).
fn anchor_onto(at: &mut Anchor, (start, end): (i16, i16)) {
    match at {
        Anchor::Start(o) if start > 0 => *o = onto_edge(*o, start),
        Anchor::End(o) if end > 0 => *o = onto_edge(*o, end),
        Anchor::Centre { before, after, .. } => {
            *before = before.saturating_add(start);
            *after = after.saturating_add(end);
        }
        _ => {}
    }
}

/// `span` re-measured from the frame's edges on its axis.
fn span_onto(span: &mut NodeSpan, strips: (i16, i16)) {
    match span {
        NodeSpan::Fixed { at, .. } | NodeSpan::Share { at, .. } => anchor_onto(at, strips),
        NodeSpan::Stretch { start, end } => {
            if strips.0 > 0 {
                *start = onto_edge(*start, strips.0);
            }
            if strips.1 > 0 {
                *end = onto_edge(*end, strips.1);
            }
        }
    }
}

/// The strips (left, top, right, bottom) between the frame rectangle
/// (`frame`, the window's insets) and the window's edge that a node's art
/// reaches into, as (moved, dropped). Owner rule (plan E7.7 TAB1): the app
/// takes the whole window and an edge node lives on the frame's edge, never
/// in a transparent strip the window reserves; lead ruling (TAB1 split):
/// - a strip only edge nodes' art lies in, each wholly inside it, is moved:
///   the nodes are re-measured onto the frame (Symbiosis's frame tiles
///   `IndentRight="13"` with its InfoView tab `MainInfoViewButton` `Z="30"
///   JustX="Right"` in that column, `MainWindow.xml:11-39`, `:199`);
/// - a strip art straddles (laid from the window's edge across the frame's)
///   is dropped: the inset is 0 and no node moves (Combat's chat corners
///   `Top*`/`Bottom*` from the window's top and bottom edges over frame rows
///   2 px inside them; its title strip stays at y 13).
///
/// A strip no art reaches into stays the pack's frame: StealthX's
/// `MainBackground` `X="112"`, the room its `MainMenuPopup` rail opens in
/// (plan SK2-S1o). A side the tree itself is inset on (`out`) is no strip.
fn edge_strips(tree: &SkinTree, frame: Insets, out: Insets) -> ([u16; 4], [u16; 4]) {
    let boxes = tree.boxes(MOST, MOST);
    let sides = [frame.left, frame.top, frame.right, frame.bottom];
    let tree_out = [out.left, out.top, out.right, out.bottom];
    let (mut moved, mut dropped) = ([0; 4], [0; 4]);
    for i in 0..4 {
        let strip = i32::from(sides[i]);
        if strip == 0 || tree_out[i] != 0 {
            continue;
        }
        // (reaches into the strip, lies wholly in it)
        let at = |b: &[i32; 4]| match i {
            0 => (b[0] < strip, b[0] + b[2] <= strip),
            1 => (b[1] < strip, b[1] + b[3] <= strip),
            2 => (b[0] + b[2] > MOST - strip, b[0] >= MOST - strip),
            _ => (b[1] + b[3] > MOST - strip, b[1] >= MOST - strip),
        };
        let art: Vec<(bool, bool)> = boxes
            .iter()
            .filter(|(n, b)| b[2] > 0 && b[3] > 0 && n.art.is_some() && shown(n))
            .map(|(_, b)| at(b))
            .filter(|(into, _)| *into)
            .collect();
        if art.is_empty() {
            continue;
        }
        if art.iter().all(|(_, within)| *within) {
            moved[i] = sides[i];
        } else {
            dropped[i] = sides[i];
        }
    }
    (moved, dropped)
}

/// Lays every root node of `tree` from the frame's edges on the sides
/// `strips` gives up: the frame's components reach the window's edge and an
/// edge node that lay in a strip sits on the frame's edge, over its border
/// (Symbiosis's tab over `RightSide`, the pack's `Z="30"` above `Z="1"`),
/// its art whole.
fn onto_frame(tree: &mut SkinTree, strips: [u16; 4]) {
    let s = strips.map(|v| i16::try_from(v).unwrap_or(i16::MAX));
    for n in &mut tree.root {
        span_onto(&mut n.x, (s[0], s[2]));
        span_onto(&mut n.y, (s[1], s[3]));
    }
}

/// A popup laid against the window's edge, re-measured from the frame's
/// edge on the sides `strips` gave up: Symbiosis's `MainInfoViewPopup`
/// (`Popups.xml:10`, `X="MainWindow.Right-13"`, where its frame ended) opens
/// from the frame's right edge, which is now the window's.
pub(super) fn popup_onto_frame(anchor: &mut PopupAnchor, strips: [u16; 4]) {
    let s = strips.map(|v| i16::try_from(v).unwrap_or(i16::MAX));
    anchor.offset = match anchor.edge {
        PopupEdge::Left => anchor.offset.saturating_sub(s[0]),
        PopupEdge::Right => anchor.offset.saturating_add(s[2]),
    };
}

/// Sizes `built`'s window to the tree (plan SK2 decision 5): the window
/// holds the tree's overhang and the frame rows' reach, the frame
/// rectangle (inset by the layout's outsets) lies where the pack places the
/// frame's components, and the smallest window is the pack's declared
/// `MinWidth`/`MinHeight` (Combat `combat.ini:3-4`, Total Chaos
/// `components.xml:4-5`), grown until every node lies inside it. A strip
/// between the frame and the window that a node's art reaches into is given
/// up first ([`edge_strips`]); returns the strips whose nodes moved, for the
/// popups' anchors.
pub(super) fn fit(
    tree: &mut SkinTree,
    pack_min: (u16, u16),
    rows: &[String],
    layout: &mut WindowLayout,
) -> Result<[u16; 4], ImportError> {
    let (over, frame) = (overhang(tree), frame_reach(tree, rows)?);
    let grow = |o: u16, f: i32| o.max(u16::try_from(f.max(0)).unwrap_or(u16::MAX));
    let out = Insets {
        left: grow(over.left, frame[0]),
        top: grow(over.top, frame[1]),
        right: grow(over.right, frame[2]),
        bottom: grow(over.bottom, frame[3]),
    };
    tree.outsets = out;
    let o = &mut layout.outsets;
    let add = |o: u16, out: u16, f: i32| {
        u16::try_from(i32::from(o) + i32::from(out) - f)
            .map_err(|_| bad("frame outsets out of range"))
    };
    o.left = add(o.left, out.left, frame[0])?;
    o.top = add(o.top, out.top, frame[1])?;
    o.right = add(o.right, out.right, frame[2])?;
    o.bottom = add(o.bottom, out.bottom, frame[3])?;
    let (given, dropped) = edge_strips(tree, *o, out);
    onto_frame(tree, given);
    o.left -= given[0] + dropped[0];
    o.top -= given[1] + dropped[1];
    o.right -= given[2] + dropped[2];
    o.bottom -= given[3] + dropped[3];
    let (ow, oh) = (
        i32::from(o.left) + i32::from(o.right),
        i32::from(o.top) + i32::from(o.bottom),
    );
    // A dropped strip keeps the window and every node where they were: the
    // frame grows to the window, so its minimum and the content's insets
    // from it grow by the strip (Combat's chat stays 304 px tall, its
    // content where its art frames it).
    let c = &mut layout.content;
    c.left += dropped[0];
    c.top += dropped[1];
    c.right += dropped[2];
    c.bottom += dropped[3];
    layout.min_width += dropped[0] + dropped[2];
    layout.min_height += dropped[1] + dropped[3];
    let (tw, th) = (
        i32::from(out.left) + i32::from(out.right),
        i32::from(out.top) + i32::from(out.bottom),
    );
    let w0 = (i32::from(layout.min_width) + ow).max(i32::from(pack_min.0) + tw);
    let h0 = (i32::from(layout.min_height) + oh).max(i32::from(pack_min.1) + th);
    let w = (w0..=MOST)
        .find(|&w| fits(tree, w, MOST, true))
        .ok_or_else(|| bad("the tree fits no window width"))?;
    let h = (h0..=MOST)
        .find(|&h| fits(tree, w, h, false))
        .ok_or_else(|| bad("the tree fits no window height"))?;
    let frame = |v: i32| u16::try_from(v).map_err(|_| bad("window too large"));
    layout.min_width = frame(w - ow)?;
    layout.min_height = frame(h - oh)?;
    Ok(given)
}

/// The views node's box as insets from the frame rectangle: the same at
/// the smallest window and one 100 px larger each way, else `None`.
#[must_use]
pub fn views_content(tree: &SkinTree, layout: &WindowLayout) -> Option<Insets> {
    let o = layout.outsets;
    let w = i32::from(layout.min_width) + i32::from(o.left) + i32::from(o.right);
    let h = i32::from(layout.min_height) + i32::from(o.top) + i32::from(o.bottom);
    let at = |w: i32, h: i32| {
        tree.boxes(w, h)
            .into_iter()
            .find(|(n, _)| n.role == NodeRole::Views)
            .map(|(_, b)| [b[0], b[1], w - b[0] - b[2], h - b[1] - b[3]])
    };
    let a = at(w, h)?;
    (at(w + 100, h + 100)? == a).then_some(())?;
    let side = |v: i32, out: u16| u16::try_from(v - i32::from(out)).ok();
    Some(Insets {
        left: side(a[0], o.left)?,
        top: side(a[1], o.top)?,
        right: side(a[2], o.right)?,
        bottom: side(a[3], o.bottom)?,
    })
}
