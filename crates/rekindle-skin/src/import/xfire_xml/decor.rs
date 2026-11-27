//! A window's layout tiles above its frame band (plan SK0 S1c, lead
//! decision): Space Cowboy's `mainwindow.xml:5,20-21` lays `SCLogo` (0,1),
//! `AGEAR` (bottom left) and `IGEAR` (bottom right) at `Z="4"` over the
//! frame pieces (`Z="1"`), and their art is the window's outline in the
//! reference. Each becomes a [`FrameTile`] with its anchors, placed in the
//! whole window (Xfire lays a root child against the window), its `Z` kept
//! as [`FrameTile::decoration`].
//!
//! A frame row's numeric `IndentRight`/`IndentLeft` is an outset: Symbiosis
//! lays `TopRow`/`BottomRow` `IndentRight="13"` (`MainWindow.xml:35,39`)
//! and its Infoview tab `MainInfoViewButton` (13×115, `JustX="Right"
//! JustY="Center"`, `:199`) in that strip, so the frame rectangle is the
//! window less 13 px on the right and the tab is a decoration there.
//!
//! A root tile is a decoration when it is above the frame (`Z` > 1), lays
//! one image unscaled (no `ResizeX`/`ResizeY="100"`, no child tiles, a
//! normal set of one image), sits at numeric offsets, and at a
//! window (every repeat laid twice) is not wholly inside the content rectangle (art wholly inside
//! the content is content, not outline: Space Cowboy's ad slot).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{FrameTile, SkinWindow, TileAxis, WindowLayout};

use super::super::tree::Node;
use super::super::views;
use super::super::ImportError;
use super::tiles::{normal_set, Shape};
use super::{descendants, nine_piece_in, row_decor, stack, tiles, Windows};
use crate::shape;

/// The names a frame row goes by (Symbiosis `TopRow`/`BottomRow`).
const ROWS: [&str; 2] = ["TopRow", "BottomRow"];

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

pub(super) fn num(tile: &Node, attr: &str) -> Option<u16> {
    tile.attr_ci(attr)
        .map_or(Some(0), |v| v.trim().parse().ok())
}

/// The outsets the frame rows' numeric indents leave (0 where a row names
/// another tile, `IndentRight="TopRight.width"` is inside the frame).
fn outsets(layout: &Node) -> (u16, u16) {
    let rows: Vec<&Node> = layout
        .children_named("Tile")
        .filter(|t| {
            t.attr_ci("Name")
                .is_some_and(|n| ROWS.iter().any(|r| eq(r, n)))
        })
        .collect();
    let side = |attr: &str| {
        rows.iter()
            .map(|t| {
                t.attr_ci(attr)
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0)
            })
            .min()
            .unwrap_or(0)
    };
    (side("IndentLeft"), side("IndentRight"))
}

fn axis(just: Option<&str>, offset: u16, far: &str) -> TileAxis {
    match just {
        Some(j) if eq(j, far) => TileAxis::End { offset },
        Some(j) if eq(j, "CENTER") || eq(j, "CENTRE") => TileAxis::Centre,
        _ => TileAxis::Start { offset },
    }
}

/// `tile`'s (anchors, Z) when it is a decoration candidate.
fn candidate(tile: &Node) -> Option<((TileAxis, TileAxis), u8)> {
    let z: u8 = tile.attr_ci("Z")?.trim().parse().ok()?;
    let resizes = ["ResizeX", "ResizeY"]
        .iter()
        .any(|a| tile.attr_ci(a) == Some("100"));
    if z <= 1 || resizes || tile.children_named("Tile").next().is_some() {
        return None;
    }
    let x = axis(tile.attr_ci("JustX"), num(tile, "X")?, "RIGHT");
    let y = axis(tile.attr_ci("JustY"), num(tile, "Y")?, "BOTTOM");
    Some(((x, y), z))
}

fn start(axis: TileAxis, len: i64, tile: i64) -> i64 {
    match axis {
        TileAxis::Start { offset } => i64::from(offset),
        TileAxis::End { offset } => len - i64::from(offset) - tile,
        TileAxis::Centre | TileAxis::Repeat { .. } => (len - tile).div_euclid(2),
    }
}

/// Whether `t` lies wholly inside `layout`'s content rectangle at a window
/// with every repeat laid twice over (the helper's second check size; at
/// the smallest frame the content can be empty).
fn inside_content(layout: &WindowLayout, t: &FrameTile) -> bool {
    let (out, inner) = (layout.outsets, layout.content);
    let span = |f: fn(&FrameTile) -> u16| {
        let most = layout.tiles.iter().map(f).max().unwrap_or(0).max(f(t));
        2 * i64::from(most) + 1
    };
    let win_w = i64::from(layout.min_width)
        + i64::from(out.left)
        + i64::from(out.right)
        + span(|t| t.width);
    let win_h = i64::from(layout.min_height)
        + i64::from(out.top)
        + i64::from(out.bottom)
        + span(|t| t.height);
    let (x, y) = (
        start(t.x, win_w, i64::from(t.width)),
        start(t.y, win_h, i64::from(t.height)),
    );
    let (cx, cy) = (
        i64::from(out.left) + i64::from(inner.left),
        i64::from(out.top) + i64::from(inner.top),
    );
    let cx1 = win_w - i64::from(out.right) - i64::from(inner.right);
    let cy1 = win_h - i64::from(out.bottom) - i64::from(inner.bottom);
    x >= cx && y >= cy && x + i64::from(t.width) <= cx1 && y + i64::from(t.height) <= cy1
}

/// Adds `layout`'s (the window's layout file) decorations and outsets to
/// `built`, each image decoded by `piece(bitmap, shift colour)`. The tree
/// draws the `stacked_rows` in their XML boxes.
///
/// # Errors
/// What `piece` returns, or an image too large for a tile.
pub fn apply(
    window: SkinWindow,
    layout: &Node,
    components: &[Node],
    stacked_rows: &[String],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
    built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>),
) -> Result<(), ImportError> {
    let (left, right) = outsets(layout);
    built.0.outsets.left = left;
    built.0.outsets.right = right;
    let mut by_name: BTreeMap<String, &Node> = BTreeMap::new();
    for c in components.iter().flat_map(|n| descendants(n, "Component")) {
        if let Some(name) = c.attr_ci("Name") {
            by_name.entry(name.to_ascii_lowercase()).or_insert(c);
        }
    }
    for tile in layout.children_named("Tile") {
        let Some((axes, z)) = candidate(tile) else {
            continue;
        };
        let (Some(name), Some(component)) = (tile.attr_ci("Name"), tile.attr_ci("Component"))
        else {
            continue;
        };
        let Some(set) = by_name
            .get(&component.to_ascii_lowercase())
            .and_then(|c| normal_set(c))
        else {
            continue;
        };
        let mut images = set.children_named("Image");
        let (Some(image), None) = (images.next(), images.next()) else {
            continue;
        };
        let img = piece(&image.text, set.attr_ci("Color"))?;
        let piece_name = format!("decor-{}", name.to_ascii_lowercase());
        let Some((mut t, png)) = shape::tile(window, &piece_name, &img, axes, true)? else {
            continue;
        };
        t.decoration = Some(z);
        if built.1.contains_key(&t.image) || inside_content(&built.0, &t) {
            continue;
        }
        built.1.insert(t.image.clone(), png);
        built.0.tiles.push(t);
    }
    for tile in layout.children_named("Tile") {
        // Compositing a stack row over itself can make its faint pixels
        // opaque in row_decor's area probe. Only the tree owns this row.
        if tile.attr_ci("Name").is_some_and(|name| {
            stacked_rows
                .iter()
                .any(|row| row.eq_ignore_ascii_case(name.trim()))
        }) {
            continue;
        }
        let Some(component) = tile
            .attr_ci("Component")
            .and_then(|c| by_name.get(&c.to_ascii_lowercase()))
        else {
            continue;
        };
        row_decor::add(window, tile, component, &piece, built)?;
    }
    Ok(())
}

/// The views tile (stretched both ways, component `TabbedControl` or
/// Symbiosis's `XfireView`) and the tiles above it, from `n` down.
fn views_path<'a>(n: &'a Node, path: &mut Vec<&'a Node>) -> bool {
    for t in n.children_named("Tile") {
        path.push(t);
        let views = t.attr_ci("ResizeX") == Some("100")
            && t.attr_ci("ResizeY") == Some("100")
            && t.attr_ci("Component")
                .is_some_and(|c| eq(c, "TabbedControl") || eq(c, "XfireView"));
        if views || views_path(t, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// The views' insets (left, right) inside the frame (plan SK0-L1,
/// [`views`]): each tile's `X` + `IndentLeft` and `IndentRight` summed
/// from below the deepest tile laying a nine-piece set (StealthX's
/// `MainBackground` is the frame), else from the layout root less the
/// frame rows' outsets (Symbiosis's `IndentRight="13"`); `None` without a
/// views tile or when a placement on the way is not a pixel count (Classic
/// v3's `X="LeftOuterEdge.RIGHT"`) or is justified to another edge.
fn views_insets(main: &Node, components: &[Node]) -> Option<(u32, u32)> {
    let mut path = Vec::new();
    if !views_path(main, &mut path) {
        return None;
    }
    let by_name: BTreeMap<String, &Node> = components
        .iter()
        .flat_map(|n| descendants(n, "Component"))
        .filter_map(|c| Some((c.attr_ci("Name")?.to_ascii_lowercase(), c)))
        .collect();
    let frame_at = path.iter().rposition(|t| {
        t.attr_ci("Component")
            .and_then(|c| by_name.get(&c.to_ascii_lowercase()))
            .and_then(|c| normal_set(c))
            .is_some_and(|set| nine_piece_in([set]).is_some())
    });
    let mut left = 0i64;
    let mut right = match frame_at {
        Some(_) => 0i64,
        None => -i64::from(outsets(main).1),
    };
    for t in &path[frame_at.map_or(0, |i| i + 1)..] {
        if t.attr_ci("JustX").is_some_and(|j| !eq(j, "LEFT")) {
            return None;
        }
        left += i64::from(num(t, "X")?) + i64::from(num(t, "IndentLeft")?);
        right += i64::from(num(t, "IndentRight")?);
    }
    Some((u32::try_from(left).ok()?, u32::try_from(right).ok()?))
}

/// The frame piece tiles [`shape::from_nine`] lays, by package path (the
/// centre is never a tile).
fn frame_piece_paths(window: SkinWindow) -> [String; 8] {
    [
        "top-left",
        "top",
        "top-right",
        "left",
        "right",
        "bottom-left",
        "bottom",
        "bottom-right",
    ]
    .map(|p| shape::tile_path(window, p))
}

/// Removes the nine-piece frame's tiles and images from `built`: Xfire
/// draws each tile once, in its own box (skin spec "a set is a frame drawn
/// in the tile's box"). [`Shape::Rows`] reads its pieces from three tiles the
/// component tree lays where the pack lays them (Classic v3's
/// `MainTitleBar` 14…W−27, `MiddleBevel` inside `InnerBevel`, `BaseBoard`
/// 10…W−25, `v3/xfiremainwindow.xml:2,16,54-57`), none in the window's box,
/// so the frame layer draws none of them again at the window's edges.
fn drop_frame_pieces(window: SkinWindow, built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>)) {
    drop_tiles(&frame_piece_paths(window), built);
}

/// Removes the frame tiles at `paths` (not a decoration) and their images.
fn drop_tiles(paths: &[String], built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>)) {
    built
        .0
        .tiles
        .retain(|t| t.decoration.is_some() || !paths.contains(&t.image));
    for p in paths {
        built.1.remove(p);
    }
}

/// The buddy list's layout from its layout file `main`: the frame pieces
/// (a nine-piece shape, else a row stack), then the decorations and
/// outsets; `None` when no frame is found.
///
/// # Errors
/// What `piece` returns, or pieces that do not compose.
pub fn main_window(
    main: &Node,
    (components, popups): (&[Node], &[Node]),
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError> + Copy,
    colours: &[super::ColourRow],
    windows: &mut Windows,
) -> Result<(), ImportError> {
    let w = SkinWindow::Main;
    let found = tiles::pieces(main, components, piece)?;
    let carried = found.as_ref().is_some_and(|(_, s, _)| *s == Shape::Rows);
    let stacked = found.is_none();
    let built = if let Some((mut pieces, _, frame_tiles)) = found {
        let insets = views_insets(main, components);
        if let Some(insets) = insets {
            let [_, _, _, l, _, r, _, _, _] = &mut pieces;
            views::to_views_edge(l, r, insets);
        }
        let mut built = shape::from_nine(w, &pieces)?;
        if let Some(insets) = insets {
            views::fill_band(w, &mut built, &pieces[4], insets)?;
        }
        // The nine pieces are the window component's own (its box), or the
        // tile laying them, placed where the pack places it (StealthX's
        // `MainBackground` `X="112"`, the rail left of it).
        Some((built, frame_tiles))
    } else {
        stack::layout(w, main, components, piece)?
    };
    if let Some((mut built, rows)) = built {
        let stacked_rows = if stacked { rows.as_slice() } else { &[] };
        apply(w, main, components, stacked_rows, piece, &mut built)?;
        super::title::add(w, main, components, colours, piece, &mut built)?;
        // The tree also reads the popups a component launches (StealthX's
        // rail); nothing else reads them.
        let mut with_popups = components.to_vec();
        with_popups.extend_from_slice(popups);
        crate::skin_tree::import::add(w, main, &with_popups, piece, &rows, &mut built)?;
        if carried {
            drop_frame_pieces(w, &mut built);
        } else if stacked {
            // The fix-1 rule for the row stack (plan SK2 lead decision 27):
            // the tree draws each row in its own box (Total Chaos's
            // `BottomBar` its top 61 rows at the window's bottom edge), so
            // the stack laid at the window's edges (`row4-*` at end 0, 5 px
            // above it) draws none of them again.
            drop_tiles(&stack::row_paths(w, rows.len()), &mut built);
        }
        windows.insert(w, built);
    }
    Ok(())
}

#[cfg(test)]
#[path = "decor_tests.rs"]
mod tests;
