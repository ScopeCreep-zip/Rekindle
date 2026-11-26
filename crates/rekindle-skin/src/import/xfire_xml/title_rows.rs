//! The titlebar strip's rows (plan E7.7.7): an Xfire main window's title
//! component may carry more art than its title. xSLIMFIRE's `MainTitleBar`
//! (`xSLIMFIREmainwindow.xml:2`, `main/top-left.gif`, `top.gif`,
//! `top-right.gif`) is 106 px tall: the title, its buttons and the menu
//! text sit in rows 0–36, and the window lays its status bar over the rows
//! below (`DownArrow` Y=37, `StatusWrapper` Y=39). Scaled whole to the
//! chrome height, that art squashes the title strip.
//!
//! The rule, from the pack's own layout (the `<Include>` under `Skin.xml`'s
//! `<MainSkin>`): the strip ends at the top of the first tile laid over the
//! title tile's rows whose component fills another slot of this import (its
//! art is that slot's, not the title's). The title buttons
//! (`titlebar.btn.*`) are the title's, and so is a tile that holds them:
//! Symbiosis lays its `InfoBar` (`buddylist.infobar`) at Y=5 over the
//! 13-row `MainTopRow`, with `ButtonMinimize`/`ButtonMaximize`/`ButtonClose`
//! inside it (`MainWindow.xml:48–57`), so that band is the title's and the
//! strip stays whole. Tiles are placed from the top
//! (`JustY` absent or `TOP`, every ancestor too) with numeric `Y`s, each
//! relative to its parent; a tile placed from the bottom or by an
//! expression (`MainTitleBar.BOTTOM-15`) has no top row to read. The rows
//! from the cut down map to no slot. A pack with no such tile keeps its
//! whole strip (Classic v3: its status bar is laid inside the bottom-placed
//! `BaseBoard`).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;

use super::super::tree::Node;
use super::super::Control;

/// A tile's component, its top row in the window, px, and whether its
/// subtree holds a title button.
struct Placed<'a> {
    component: &'a str,
    top: i64,
    titled: bool,
}

/// Whether `node`'s subtree lays a tile of one of `buttons`.
fn holds(node: &Node, buttons: &[&String]) -> bool {
    node.children.iter().any(|c| {
        c.name.eq_ignore_ascii_case("Tile")
            && (c
                .attr_ci("Component")
                .is_some_and(|n| buttons.iter().any(|b| b.eq_ignore_ascii_case(n)))
                || holds(c, buttons))
    })
}

/// Every tile of `node`'s subtree placed from the top, with its absolute
/// top row (`parent` is the node's own).
fn placed<'a>(node: &'a Node, parent: i64, buttons: &[&String], out: &mut Vec<Placed<'a>>) {
    for tile in node
        .children
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case("Tile"))
    {
        let from_top = tile
            .attr_ci("JustY")
            .is_none_or(|j| j.eq_ignore_ascii_case("TOP"));
        let y = tile
            .attr_ci("Y")
            .map_or(Some(0), |y| y.trim().parse::<i64>().ok());
        let (true, Some(y)) = (from_top, y) else {
            continue;
        };
        let top = parent + y;
        if let Some(component) = tile.attr_ci("Component") {
            let titled = holds(tile, buttons);
            out.push(Placed {
                component,
                top,
                titled,
            });
        }
        placed(tile, top, buttons, out);
    }
}

/// The row the titlebar strip ends at in `layout` (the main window's
/// layout file), `None` when no other slot's tile is laid over it.
#[must_use]
pub fn cut(layout: &Node, sources: &BTreeMap<&'static str, String>, height: u32) -> Option<u32> {
    let title = sources.get("titlebar")?;
    let buttons: Vec<&String> = sources
        .iter()
        .filter(|(region, _)| region.starts_with("titlebar."))
        .map(|(_, name)| name)
        .collect();
    let mut tiles = Vec::new();
    if layout.name.eq_ignore_ascii_case("Tile") {
        if let Some(component) = layout.attr_ci("Component") {
            tiles.push(Placed {
                component,
                top: 0,
                titled: true,
            });
        }
    }
    placed(layout, 0, &buttons, &mut tiles);
    let top = tiles
        .iter()
        .find(|t| t.component.eq_ignore_ascii_case(title))?
        .top;
    let others: Vec<&String> = sources
        .iter()
        .filter(|(region, name)| {
            !region.starts_with("titlebar") && !name.eq_ignore_ascii_case(title)
        })
        .map(|(_, name)| name)
        .collect();
    tiles
        .iter()
        .filter(|t| !t.titled && others.iter().any(|o| o.eq_ignore_ascii_case(t.component)))
        .filter_map(|t| u32::try_from(t.top - top).ok())
        .filter(|row| (1..height).contains(row))
        .min()
}

/// Crops the titlebar strip in `controls` to the rows [`cut`] finds in
/// `layout`; the row it ends at, `None` when the strip is kept whole.
pub fn crop(
    layout: &Node,
    controls: &mut [Control],
    sources: &BTreeMap<&'static str, String>,
) -> Option<u32> {
    let strip = controls.iter_mut().find(|c| c.region == "titlebar")?;
    let height = strip.images.values().map(RgbaImage::height).min()?;
    let rows = cut(layout, sources, height)?;
    for img in strip.images.values_mut() {
        *img = RgbaImage::from_fn(img.width(), rows, |x, y| *img.get_pixel(x, y));
    }
    Some(rows)
}

#[cfg(test)]
#[path = "title_rows_tests.rs"]
mod tests;
