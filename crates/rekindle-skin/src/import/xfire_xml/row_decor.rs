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
mod tests {
    use super::super::super::tree::parse;
    use super::super::decor::apply;
    use super::*;

    /// Onyx's `MenuBar` across a window whose side pieces are transparent
    /// (kept: it fills the side columns), and a row over the opaque top
    /// row (dropped: it adds nothing to the outline).
    #[test]
    fn a_row_is_kept_only_where_it_adds_to_the_outline() {
        let layout = parse(
            "mainwindow.xml",
            br#"<Tile Name="Main" Component="MainWindow">
<Tile Name="MenuBar" X="0" Y="12" Z="1" ResizeX="100" IndentRight="0" Component="Bar"/>
<Tile Name="Over" X="0" Y="0" Z="1" ResizeX="100" Component="Bar"/></Tile>"#,
        )
        .unwrap();
        let components = parse(
            "components.xml",
            br#"<Components><Component Name="Bar"><Images>
<Image Type="left">l</Image><Image Type="middle">m</Image><Image Type="right">r</Image>
</Images></Component></Components>"#,
        )
        .unwrap();
        let px = |a| rekindle_image::Rgba([1, 2, 3, a]);
        let pieces = std::array::from_fn(|i| {
            RgbaImage::from_pixel(10, 10, px(if i == 3 || i == 5 { 0 } else { 255 }))
        });
        let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
        apply(
            SkinWindow::Main,
            &layout,
            &[components],
            |_, _| Ok(RgbaImage::from_pixel(10, 2, px(255))),
            &mut built,
        )
        .unwrap();
        let got: Vec<_> = built
            .0
            .tiles
            .iter()
            .filter(|t| t.decoration.is_some())
            .map(|t| (t.image.as_str(), t.x, t.y))
            .collect();
        let at = |part: &str| format!("assets/frame/main/row-menubar-{part}.png");
        let y = TileAxis::Start { offset: 12 };
        let (l, m, r) = (at("left"), at("middle"), at("right"));
        let want = vec![
            (l.as_str(), TileAxis::Start { offset: 0 }, y),
            (m.as_str(), TileAxis::Repeat { start: 10, end: 10 }, y),
            (r.as_str(), TileAxis::End { offset: 0 }, y),
        ];
        assert_eq!(got, want);
    }

    /// xSLIMFIRE's `AdArea` row names `ADBARLEFT`/`ADBARRIGHT`, which its
    /// theme never defines: the row lays its middle only and the import
    /// goes on; a defined bitmap whose file is missing still stops it.
    #[test]
    fn an_undefined_side_bitmap_lays_nothing_and_a_missing_file_stops() {
        let layout = parse(
            "mainwindow.xml",
            br#"<Tile Name="Main" Component="MainWindow">
<Tile Name="AdArea" X="0" Y="12" Z="1" ResizeX="100" IndentRight="0" Component="Bar"/></Tile>"#,
        )
        .unwrap();
        let components = parse(
            "components.xml",
            br#"<Components><Component Name="Bar"><Images>
<Image Type="left">ADBARLEFT</Image><Image Type="middle">BLACK</Image><Image Type="right">ADBARRIGHT</Image>
</Images></Component></Components>"#,
        )
        .unwrap();
        let px = |a| rekindle_image::Rgba([1, 2, 3, a]);
        let pieces = std::array::from_fn(|i| {
            RgbaImage::from_pixel(10, 10, px(if i == 3 || i == 5 { 0 } else { 255 }))
        });
        let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
        let undefined = |name: &str, _: Option<&str>| match name {
            "BLACK" => Ok(RgbaImage::from_pixel(10, 2, px(255))),
            _ => Err(ImportError::UndefinedBitmap {
                name: name.to_owned(),
            }),
        };
        apply(
            SkinWindow::Main,
            &layout,
            std::slice::from_ref(&components),
            undefined,
            &mut built,
        )
        .unwrap();
        let got: Vec<_> = built
            .0
            .tiles
            .iter()
            .filter(|t| t.decoration.is_some())
            .map(|t| t.image.as_str())
            .collect();
        assert_eq!(got, ["assets/frame/main/row-adarea-middle.png"]);
        let missing = |name: &str, _: Option<&str>| match name {
            "BLACK" => Ok(RgbaImage::from_pixel(10, 2, px(255))),
            _ => Err(ImportError::MissingFile {
                file: format!("main/{name}.gif"),
            }),
        };
        let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
        assert!(matches!(
            apply(
                SkinWindow::Main,
                &layout,
                &[components],
                missing,
                &mut built
            ),
            Err(ImportError::MissingFile { .. })
        ));
    }

    /// Space Cowboy's `BRGripper`: stretched (`ResizeX="100"`) but capped by
    /// its component's `MaxWidth="38"`, so it is no row (laid across a wide
    /// window its dots made 4499 silhouette rectangles).
    #[test]
    fn a_stretch_its_component_caps_is_no_row() {
        let layout = parse(
            "mainwindow.xml",
            br#"<Tile Name="Main" Component="MainWindow">
<Tile Name="BRGripper" JustX="RIGHT" X="19" JustY="BOTTOM" Y="2" ResizeX="100" Z="40" Component="BRGripper"/></Tile>"#,
        )
        .unwrap();
        let components = parse(
            "components.xml",
            br#"<Components><Component Name="BRGripper" Class="Tile" MinWidth="38" MaxWidth="38"><Images>
<Image Type="middle">grip</Image></Images></Component></Components>"#,
        )
        .unwrap();
        let px = |a| rekindle_image::Rgba([1, 2, 3, a]);
        let pieces = std::array::from_fn(|i| {
            RgbaImage::from_pixel(10, 10, px(if i == 3 || i == 5 || i == 7 { 0 } else { 255 }))
        });
        let mut built = shape::from_nine(SkinWindow::Main, &pieces).unwrap();
        apply(
            SkinWindow::Main,
            &layout,
            &[components],
            |_, _| Ok(RgbaImage::from_pixel(10, 2, px(255))),
            &mut built,
        )
        .unwrap();
        assert!(
            built.0.tiles.iter().all(|t| t.decoration.is_none()),
            "{:?}",
            built.0.tiles
        );
    }
}
