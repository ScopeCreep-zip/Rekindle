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

/// One component tree's rules (plan SK2): a window kind's tree, or a popup
/// it launches (`popup` its name), apart from the shared frame rules so
/// each tree's art is served on its own shell (plan SK4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSheet {
    pub window: SkinWindow,
    pub popup: Option<String>,
    pub css: String,
}

/// The root a window kind's title strip lays: only its own framed windows
/// (plan SK5, lead decision): the buddy list's strip lays
/// `data-skin-window="main"` alone, so a pack with no chat strip never
/// lays its logo tile and window buttons over the chat window's header
/// (Symbiosis's 18 x 16 buttons over call and video); that chat window
/// keeps the app's titlebar inside its pack frame. Never an unframed
/// window (no `data-skin-window`).
fn title_root(window: SkinWindow) -> String {
    format!(".app-frame[data-skin-window=\"{}\"]", window_attr(window))
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
    let (shared, trees) = frame_parts(manifest, images, base)?;
    Some(layered(&shared, &trees))
}

/// [`frame_css`]'s rules wrapped in its `@layer shape` (empty when the
/// skin lays no window).
#[must_use]
pub fn layered(shared: &str, trees: &[TreeSheet]) -> String {
    if shared.is_empty() && trees.is_empty() {
        return String::new();
    }
    let mut out = String::from("@layer shape {\n");
    out.push_str(shared);
    for t in trees {
        out.push_str(&t.css);
    }
    out.push_str("}\n");
    out
}

/// [`frame_css`] in its parts: the rules every window shares (frames and
/// title strips) and each component tree's own rules, in emission order.
/// `None` as [`frame_css`].
#[must_use]
pub fn frame_parts(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<(String, Vec<TreeSheet>)> {
    let mut out = String::new();
    for (&window, layout) in &manifest.windows {
        window_rules(&mut out, window, layout, images, base)?;
    }
    // The pack-laid title strips (plan SK1-S1), only in the windows the
    // pack frames (the desktop's `frame_of`: the buddy list and chat, the
    // roots carrying `data-skin-window`); every other window (Settings, the
    // skin editor, a popup) keeps the app's own titlebar. Each strip lays
    // only its own window kind: a chat window gets a pack strip only from
    // the pack's chat strip (plan SK5).
    for (&window, layout) in &manifest.windows {
        let Some(title) = &layout.title else { continue };
        let top = layout.outsets.top.saturating_add(layout.content.top);
        out.push_str(&super::title::title_css(
            &title_root(window),
            title,
            top,
            images,
            base,
        )?);
    }
    // The pack's component trees (plan SK2): one rule per node.
    let mut trees = Vec::new();
    for (&window, layout) in &manifest.windows {
        let Some(tree) = &layout.tree else { continue };
        let root = format!(".app-frame[data-skin-window=\"{}\"]", window_attr(window));
        trees.push(TreeSheet {
            window,
            popup: None,
            css: crate::skin_tree::css::tree_css(&root, tree, layout, images, base)?,
        });
        // Each popup the window launches: its frame carries
        // `data-skin-popup="<name>"` (plan SK2 P1).
        for (name, popup) in &layout.popups {
            let root = format!(".app-frame[data-skin-popup=\"{name}\"]");
            let frame = crate::skin_tree::popup::frame(popup, layout.min_height);
            trees.push(TreeSheet {
                window,
                popup: Some(name.clone()),
                css: crate::skin_tree::css::tree_css(&root, &popup.tree, &frame, images, base)?,
            });
        }
    }
    Some((out, trees))
}

/// The smallest window (logical px) that holds `layout`: its `min_*` (the
/// smallest frame its tiles fit, plan SK0 S1) grown by its `outsets`, as
/// the window is the frame grown by them. The one source of the minimum
/// the Host serves on a shell and the desktop's fit applies (plan SK4).
/// `None` past `u16` (never for a validated package: [`super::check`]
/// bounds a window by [`super::MAX_WINDOW_WIDTH`]/[`super::MAX_WINDOW_HEIGHT`]).
#[must_use]
pub fn min_window(layout: &WindowLayout) -> Option<(u16, u16)> {
    let o = &layout.outsets;
    Some((
        layout.min_width.checked_add(o.left)?.checked_add(o.right)?,
        layout
            .min_height
            .checked_add(o.top)?
            .checked_add(o.bottom)?,
    ))
}

#[cfg(test)]
#[path = "css_tests.rs"]
mod tests;
