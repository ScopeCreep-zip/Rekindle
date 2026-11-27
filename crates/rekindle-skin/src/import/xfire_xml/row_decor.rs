//! Row decorations (plan SK0 S1f): a root tile stretched across the window
//! less its numeric indents (`ResizeX="100"`, numeric `X`, `Y`,
//! `IndentRight`) whose component lays a left, a middle and a right image.
//! Onyx's `MenuBar` (`mainwindow.xml` `X="5" Y="110" Z="1" ResizeX="100"
//! IndentRight="5"`, `components.xml:246-255` menubar-left/middle/right)
//! fills the rows y 110 to 127 that its frame pieces leave transparent;
//! without it the window is cut in two there. It becomes three tiles: the
//! left at `X`, the middle repeated from `X` + the left's width to
//! `IndentRight` + the right's width, the right at `IndentRight` from the
//! right edge (Xfire's `Tile` lays a resizing component's middle between
//! its ends; research §7.2), each a decoration with the tile's `Z`.
//!
//! A row is kept only when it adds pixels to the window's outline: most
//! such rows (Onyx's `StatusWrapper`) lie over the content or over opaque
//! frame, and keeping them would only cost package bytes and tiles against
//! the helper's cap (`shape::check` rule 5).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{FrameTile, SkinWindow, TileAxis, WindowLayout};

use super::super::tree::Node;
use super::super::ImportError;
use super::decor::num;
use super::tiles::normal_set;
use crate::shape::{self, SCALE_ONE};

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `tile`'s (`X`, y axis, `IndentRight`, `Z`) when it is a row: stretched
/// across, not down, at numeric offsets.
fn candidate(tile: &Node) -> Option<(u16, TileAxis, u16, u8)> {
    let across = tile.attr_ci("ResizeX") == Some("100");
    if !across || tile.attr_ci("ResizeY") == Some("100") {
        return None;
    }
    let (x, y, indent) = (num(tile, "X")?, num(tile, "Y")?, num(tile, "IndentRight")?);
    let y = match tile.attr_ci("JustY") {
        Some(j) if eq(j, "BOTTOM") => TileAxis::End { offset: y },
        _ => TileAxis::Start { offset: y },
    };
    let z = tile
        .attr_ci("Z")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    Some((x, y, indent, u8::try_from(z.clamp(0, 255)).unwrap_or(0)))
}

/// The window's outline area, px, at a window with every repeat laid twice
/// over (the helper's second check size).
fn area(layout: &WindowLayout, files: &BTreeMap<String, Vec<u8>>) -> Result<u64, ImportError> {
    let span =
        |f: fn(&FrameTile) -> u16| 2 * u32::from(layout.tiles.iter().map(f).max().unwrap_or(0)) + 1;
    let o = layout.outsets;
    let width = u32::from(layout.min_width) + u32::from(o.left) + u32::from(o.right);
    let height = u32::from(layout.min_height) + u32::from(o.top) + u32::from(o.bottom);
    let fail = |detail: String| ImportError::Image {
        file: shape::tile_path(SkinWindow::Main, "frame"),
        detail,
    };
    let masks = shape::of_layout("windows", layout, files).map_err(|e| fail(e.to_string()))?;
    let rects = shape::silhouette(
        layout,
        &masks,
        width + span(|t| t.width),
        height + span(|t| t.height),
        SCALE_ONE,
    )
    .map_err(|e| fail(format!("{e:?}")))?;
    Ok(rects
        .iter()
        .map(|r| u64::from(r.width) * u64::from(r.height))
        .sum())
}

/// Adds `tile`'s row to `built` when it is one and adds pixels to the
/// outline; each image decoded by `piece(bitmap, shift colour)`.
///
/// # Errors
/// What `piece` returns, or an image too large for a tile.
pub(super) fn add(
    window: SkinWindow,
    tile: &Node,
    component: &Node,
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
    built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>),
) -> Result<(), ImportError> {
    let (Some((x, y, indent, z)), Some(name), Some(set)) =
        (candidate(tile), tile.attr_ci("Name"), normal_set(component))
    else {
        return Ok(());
    };
    // A stretch its component caps (`MaxWidth`) never spans the window, so
    // it is no row: Space Cowboy's `BRGripper` (`mainwindow.xml:89`
    // `ResizeX="100" JustX="RIGHT" X="19"`, `components.xml:1354-1359`
    // `MinWidth="38" MaxWidth="38"`) is one 38 px grip in the bottom right
    // corner (the tree's `Resize(BottomRight)` node draws its art), not 38 px
    // dots repeated across the window.
    if component.attr_ci("MaxWidth").is_some() {
        return Ok(());
    }
    // A piece naming a bitmap the pack never defines lays nothing, as in
    // Xfire (xSLIMFIRE's `AdArea` left/right `ADBARLEFT`/`ADBARRIGHT`); a
    // defined bitmap whose file is missing still stops the import.
    let image = |ty: &str| -> Result<Option<RgbaImage>, ImportError> {
        let Some(i) = set
            .children_named("Image")
            .find(|i| i.attr_ci("Type").is_some_and(|t| eq(t, ty)))
        else {
            return Ok(None);
        };
        match piece(&i.text, set.attr_ci("Color")) {
            Err(ImportError::UndefinedBitmap { .. }) => Ok(None),
            other => other.map(Some),
        }
    };
    let (Some(middle), left, right) = (image("middle")?, image("left")?, image("right")?) else {
        return Ok(());
    };
    let width = |img: &Option<RgbaImage>| img.as_ref().map_or(0, RgbaImage::width);
    let file = shape::tile_path(window, "frame");
    let axes = [
        TileAxis::Start { offset: x },
        TileAxis::Repeat {
            start: shape::u16_of(&file, u32::from(x) + width(&left))?,
            end: shape::u16_of(&file, u32::from(indent) + width(&right))?,
        },
        TileAxis::End { offset: indent },
    ];
    let before = area(&built.0, &built.1)?;
    let mut with = built.clone();
    let row = name.to_ascii_lowercase();
    for (part, (img, axis)) in ["left", "middle", "right"].into_iter().zip(
        [left.as_ref(), Some(&middle), right.as_ref()]
            .into_iter()
            .zip(axes),
    ) {
        let Some(img) = img else {
            continue;
        };
        let piece_name = format!("row-{row}-{part}");
        let Some((mut t, png)) = shape::tile(window, &piece_name, img, (axis, y), true)? else {
            continue;
        };
        if with.1.contains_key(&t.image) {
            return Ok(());
        }
        t.decoration = Some(z);
        with.1.insert(t.image.clone(), png);
        with.0.tiles.push(t);
    }
    if area(&with.0, &with.1)? > before {
        *built = with;
    }
    Ok(())
}

#[cfg(test)]
#[path = "row_decor_tests.rs"]
mod tests;
