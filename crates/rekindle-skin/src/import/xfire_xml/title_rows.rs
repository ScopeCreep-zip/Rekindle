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
//! art is that slot's, not the title's); the title buttons
//! (`titlebar.btn.*`, Symbiosis's `ButtonMinimize` at Y=5) are the title's. Tiles are placed from the top
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

/// A tile's component and its top row in the window, px.
struct Placed<'a> {
    component: &'a str,
    top: i64,
}

/// Every tile of `node`'s subtree placed from the top, with its absolute
/// top row (`parent` is the node's own).
fn placed<'a>(node: &'a Node, parent: i64, out: &mut Vec<Placed<'a>>) {
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
            out.push(Placed { component, top });
        }
        placed(tile, top, out);
    }
}

/// The row the titlebar strip ends at in `layout` (the main window's
/// layout file), `None` when no other slot's tile is laid over it.
#[must_use]
pub fn cut(layout: &Node, sources: &BTreeMap<&'static str, String>, height: u32) -> Option<u32> {
    let title = sources.get("titlebar")?;
    let mut tiles = Vec::new();
    if layout.name.eq_ignore_ascii_case("Tile") {
        if let Some(component) = layout.attr_ci("Component") {
            tiles.push(Placed { component, top: 0 });
        }
    }
    placed(layout, 0, &mut tiles);
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
        .filter(|t| others.iter().any(|o| o.eq_ignore_ascii_case(t.component)))
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
mod tests {
    use super::*;
    use crate::import::tree;

    fn sources(pairs: &[(&'static str, &str)]) -> BTreeMap<&'static str, String> {
        pairs.iter().map(|(r, n)| (*r, (*n).to_owned())).collect()
    }

    /// xSLIMFIRE's layout, cut to the tiles the rule reads.
    const SLIM: &str = r#"<Tile Name="W" Component="MainWindow">
        <Tile Name="MainTitleBar" X="0" Y="0" Component="MainTitleBar">
            <Tile Name="XfireLogo" X="20" Y="38" Component="XfireLogo"></Tile>
            <Tile Name="CloseBox" JustX="RIGHT" X="19" Y="4" Component="CloseBox" />
        </Tile>
        <Tile Name="BaseBoard" Y="0" JustY="BOTTOM" Component="BaseBoard">
            <Tile Name="AddButton" Y="5" Component="AddButton"></Tile>
        </Tile>
        <Tile Name="StatusWrapper" X="96" Y="39" Component="StatusWrapper"></Tile>
        <Tile Name="DownArrow" X="21" JustX="RIGHT" Y="37" Component="DownArrow"></Tile>
        <Tile Name="Gripper" Y="MainTitleBar.BOTTOM-15" Component="BottomBar"></Tile>
    </Tile>"#;

    fn slim() -> Node {
        tree::parse("xSLIMFIREmainwindow.xml", SLIM.as_bytes()).expect("xml")
    }

    #[test]
    fn the_strip_ends_where_another_slots_art_is_laid() {
        let s = sources(&[
            ("titlebar", "MainTitleBar"),
            ("titlebar.btn.close", "CloseBox"),
            ("statusbar", "StatusWrapper"),
            ("statusbar.arrow", "DownArrow"),
            ("buddylist.action-button", "AddButton"),
            ("buddylist.actionbar", "BottomBar"),
        ]);
        assert_eq!(cut(&slim(), &s, 106), Some(37));
    }

    #[test]
    fn unmapped_art_and_bottom_placed_tiles_keep_the_whole_strip() {
        // The logo maps to no slot; AddButton sits in a bottom-placed tile;
        // the gripper's Y is an expression.
        // The title button at Y=4 is the title's own.
        let s = sources(&[
            ("titlebar", "MainTitleBar"),
            ("titlebar.btn.close", "CloseBox"),
            ("buddylist.action-button", "AddButton"),
            ("buddylist.actionbar", "BottomBar"),
        ]);
        assert_eq!(cut(&slim(), &s, 106), None);
    }

    #[test]
    fn crop_keeps_the_title_rows_of_every_state() {
        let s = sources(&[
            ("titlebar", "MainTitleBar"),
            ("statusbar.arrow", "DownArrow"),
        ]);
        let img = RgbaImage::from_fn(30, 106, |_, y| {
            rekindle_image::Rgba([0, 0, 0, u8::try_from(y).unwrap_or(0)])
        });
        let mut controls = vec![Control {
            region: "titlebar",
            fit: rekindle_types::dto::skin::RegionFit::Stretch,
            images: [
                (rekindle_types::dto::skin::RegionState::Normal, img.clone()),
                (rekindle_types::dto::skin::RegionState::WindowInactive, img),
            ]
            .into(),
        }];
        assert_eq!(crop(&slim(), &mut controls, &s), Some(37));
        for img in controls[0].images.values() {
            assert_eq!(img.dimensions(), (30, 37));
            assert_eq!(img.get_pixel(0, 36).0[3], 36);
        }
    }
}
