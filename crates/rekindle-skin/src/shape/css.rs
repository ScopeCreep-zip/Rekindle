//! The window's drawn outline (plan SK0 S2): each window kind's frame
//! tiles as positioned `skin://` images, served with the skin's stylesheet.
//!
//! One element per tile (`SkinFrame`'s `<i>` slots), not one element's
//! background layers: a `Repeat { start, end }` tile stops `end` px before
//! its box's edge with the last copy cut, and a background layer repeats
//! over its whole painting area (`background-clip` takes only box
//! keywords), so a corner's transparent pixels would show the strip under
//! them. Each slot's box clips its own repeat; no per-copy DOM and no
//! script lays anything out.
//!
//! The rules live in the cascade layer `shape`, after `components` (the
//! frame replaces `.app-frame`'s box) and before `a11y` and `sec`
//! (`src/styles/global.css`). The tiles are placed with the arithmetic of
//! [`super::silhouette`]'s `copies`, so the outline drawn is the region the
//! platform shaper cuts (S3): a Centre tile is floored to a whole pixel
//! like `div_euclid(2)`, and every image is drawn at its source size,
//! never stretched.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::{
    FrameTile, Insets, SkinManifest, SkinWindow, TileAxis, WindowLayout,
};

use crate::regions::css::href;

/// The most tiles a window layout may have: the `<i>` slots `SkinFrame`
/// renders (`src/components/common/SkinFrame.tsx` `FRAME_TILES`). The
/// built-ins use at most 15.
pub const MAX_FRAME_TILES: usize = 32;

/// The `data-skin-window` value of a window kind's root.
#[must_use]
pub const fn window_attr(window: SkinWindow) -> &'static str {
    match window {
        SkinWindow::Main => "main",
        SkinWindow::Chat => "chat",
    }
}

/// The slots in paint order: frame pieces as laid, then decorations by
/// their Z (ties in layout order).
#[must_use]
pub fn paint_order(layout: &WindowLayout) -> Vec<usize> {
    let mut frame: Vec<usize> = (0..layout.tiles.len())
        .filter(|&i| layout.tiles[i].decoration.is_none())
        .collect();
    let mut decor: Vec<(u8, usize)> = layout
        .tiles
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.decoration.map(|z| (z, i)))
        .collect();
    decor.sort_unstable();
    frame.extend(decor.into_iter().map(|(_, i)| i));
    frame
}

/// One axis of a tile: its declarations and whether it repeats.
/// `near`/`far` are the box's insets from the window's edges on that axis,
/// `(start, end, size)` the CSS properties (`left`, `right`, `width`).
fn axis(
    a: TileAxis,
    (near, far): (u16, u16),
    (start, end, size): (&str, &str, &str),
    tile: u16,
) -> (String, bool) {
    let (near, far, tile) = (u32::from(near), u32::from(far), u32::from(tile));
    match a {
        TileAxis::Start { offset } => (
            format!("{start}: {}px; {size}: {tile}px;", near + u32::from(offset)),
            false,
        ),
        TileAxis::End { offset } => (
            format!("{end}: {}px; {size}: {tile}px;", far + u32::from(offset)),
            false,
        ),
        TileAxis::Centre => (
            format!(
                "{start}: calc({near}px + round(down, calc((100% - {}px) / 2), 1px)); {size}: {tile}px;",
                near + far + tile
            ),
            false,
        ),
        TileAxis::Repeat { start: a, end: b } => (
            format!(
                "{start}: {}px; {end}: {}px;",
                near + u32::from(a),
                far + u32::from(b)
            ),
            true,
        ),
    }
}

/// The rule body of tile `t` (frame rectangle inset by `o` from the
/// window, or the window itself for an overhang) at `href`.
fn tile_decls(t: &FrameTile, o: Insets, href: &str) -> String {
    let o = if t.overhang {
        Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        }
    } else {
        o
    };
    let (x, rx) = axis(t.x, (o.left, o.right), ("left", "right", "width"), t.width);
    let (y, ry) = axis(
        t.y,
        (o.top, o.bottom),
        ("top", "bottom", "height"),
        t.height,
    );
    let repeat = match (rx, ry) {
        (true, true) => "repeat",
        (true, false) => "repeat-x",
        (false, true) => "repeat-y",
        (false, false) => "no-repeat",
    };
    format!(
        "display: block; {x} {y} background: url(\"{href}\") 0 0 / {}px {}px {repeat};",
        t.width, t.height
    )
}

fn window_rules(
    out: &mut String,
    window: SkinWindow,
    layout: &WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<()> {
    if layout.tiles.len() > MAX_FRAME_TILES {
        return None;
    }
    let root = format!(".app-frame[data-skin-window=\"{}\"]", window_attr(window));
    let (o, c) = (layout.outsets, layout.content);
    let pad = |a: u16, b: u16| u32::from(a) + u32::from(b);
    let _ = writeln!(
        out,
        "  {root} {{\n    border: 0;\n    border-radius: 0;\n    border-image: none;\n    position: relative;\n    isolation: isolate;\n    padding: {}px {}px {}px {}px;\n    background: var(--color-xfire-bg-dark) content-box;\n    --skin-region-titlebar-title-backing: var(--skin-region-titlebar-backing);\n  }}",
        pad(o.top, c.top),
        pad(o.right, c.right),
        pad(o.bottom, c.bottom),
        pad(o.left, c.left),
    );
    let _ = writeln!(out, "  {root} > .skin-frame {{\n    display: block;\n  }}");
    for (slot, i) in paint_order(layout).into_iter().enumerate() {
        let t = &layout.tiles[i];
        let h = href(base, &t.image, images)?;
        let _ = writeln!(
            out,
            "  {root} > .skin-frame > :nth-child({}) {{\n    {}\n  }}",
            slot + 1,
            tile_decls(t, o, &h)
        );
    }
    Some(())
}

/// The frame rules of every window kind `manifest` lays, images under
/// `base` (the Host's serving origin and path); empty when it lays none.
/// `None` for a tile image `images` lacks, a layout past
/// [`MAX_FRAME_TILES`] or an image that is neither PNG nor GIF (never for
/// a validated package).
#[must_use]
pub fn frame_css(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<String> {
    if manifest.windows.is_empty() {
        return Some(String::new());
    }
    let mut out = String::from("@layer shape {\n");
    for (&window, layout) in &manifest.windows {
        window_rules(&mut out, window, layout, images, base)?;
    }
    // The pack-laid title strips (plan SK1-S1): the buddy list's lays every
    // window without its own (Xfire's dialogs and windows share the pack's
    // title buttons); a window kind with its own strip lays its windows.
    let own_chat = manifest
        .windows
        .get(&SkinWindow::Chat)
        .is_some_and(|l| l.title.is_some());
    for (&window, layout) in &manifest.windows {
        let Some(title) = &layout.title else { continue };
        let root = match (window, own_chat) {
            (SkinWindow::Main, true) => ".app-frame:not([data-skin-window=\"chat\"])".to_owned(),
            (SkinWindow::Main, false) => ".app-frame".to_owned(),
            (SkinWindow::Chat, _) => {
                format!(".app-frame[data-skin-window=\"{}\"]", window_attr(window))
            }
        };
        let top = layout.outsets.top.saturating_add(layout.content.top);
        out.push_str(&super::title::title_css(&root, title, top, images, base)?);
    }
    // The pack's component trees (plan SK2): one rule per node.
    for (&window, layout) in &manifest.windows {
        let Some(tree) = &layout.tree else { continue };
        let root = format!(".app-frame[data-skin-window=\"{}\"]", window_attr(window));
        out.push_str(&crate::skin_tree::css::tree_css(
            &root, tree, layout, images, base,
        )?);
        // Each popup the window launches: its frame carries
        // `data-skin-popup="<name>"` (plan SK2 P1).
        for (name, popup) in &layout.popups {
            let root = format!(".app-frame[data-skin-popup=\"{name}\"]");
            let frame = crate::skin_tree::popup::frame(popup, layout.min_height);
            out.push_str(&crate::skin_tree::css::tree_css(
                &root,
                &popup.tree,
                &frame,
                images,
                base,
            )?);
        }
    }
    out.push_str("}\n");
    Some(out)
}

#[cfg(test)]
#[path = "css_tests.rs"]
mod tests;
