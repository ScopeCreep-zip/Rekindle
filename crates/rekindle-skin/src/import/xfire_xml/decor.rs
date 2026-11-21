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
use super::tiles::normal_set;
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
/// `built`, each image decoded by `piece(bitmap, shift colour)`.
///
/// # Errors
/// What `piece` returns, or an image too large for a tile.
pub fn apply(
    window: SkinWindow,
    layout: &Node,
    components: &[Node],
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

/// The buddy list's layout from its layout file `main`: the frame pieces
/// (a nine-piece shape, else a row stack), then the decorations and
/// outsets; `None` when no frame is found.
///
/// # Errors
/// What `piece` returns, or pieces that do not compose.
pub fn main_window(
    main: &Node,
    components: &[Node],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError> + Copy,
    windows: &mut Windows,
) -> Result<(), ImportError> {
    let w = SkinWindow::Main;
    let built = if let Some((mut pieces, _)) = tiles::pieces(main, components, piece)? {
        if let Some(insets) = views_insets(main, components) {
            let [_, _, _, l, _, r, _, _, _] = &mut pieces;
            views::to_views_edge(l, r, insets);
        }
        Some(shape::from_nine(w, &pieces)?)
    } else {
        stack::layout(w, main, components, piece)?
    };
    if let Some(mut built) = built {
        apply(w, main, components, piece, &mut built)?;
        windows.insert(w, built);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::tree::parse;
    use super::*;

    /// Symbiosis's rows `IndentRight="13"` and Infoview tab, Space Cowboy's
    /// logo, and an ad slot wholly inside the content.
    #[test]
    fn indents_are_outsets_and_tiles_above_the_frame_are_decorations() {
        let layout = parse(
            "MainWindow.xml",
            br#"<Tile Name="MainWindow" Component="MainWindow">
<Tile Name="TopRow" Z="1" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="BottomRow" Z="1" JustY="Bottom" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="InfoTab" Z="30" JustX="Right" JustY="Center" Component="Tab"/>
<Tile Name="SCLogo" Z="4" X="0" Y="1" Component="Logo"/>
<Tile Name="Ad" Z="10" JustX="CENTER" Y="12" Component="Ad"/>
<Tile Name="Edge" Z="1" Component="Logo"/></Tile>"#,
        )
        .unwrap();
        let components = parse(
            "Components/MainWindow.xml",
            br#"<Components>
<Component Name="Tab"><Images State="Normal"><Image Type="Middle">tab</Image></Images></Component>
<Component Name="Logo"><Images><Image Type="Middle">logo</Image></Images></Component>
<Component Name="Ad"><Images><Image Type="Middle">ad</Image></Images></Component>
</Components>"#,
        )
        .unwrap();
        let solid = |w, h| RgbaImage::from_pixel(w, h, rekindle_image::Rgba([1, 2, 3, 255]));
        let mut built =
            shape::from_nine(SkinWindow::Main, &std::array::from_fn(|_| solid(10, 10))).unwrap();
        apply(
            SkinWindow::Main,
            &layout,
            &[components],
            |name, _| {
                Ok(match name {
                    "tab" => solid(13, 20),
                    "logo" => solid(30, 8),
                    _ => solid(2, 2),
                })
            },
            &mut built,
        )
        .unwrap();
        assert_eq!((built.0.outsets.left, built.0.outsets.right), (0, 13));
        let got: Vec<_> = built
            .0
            .tiles
            .iter()
            .filter_map(|t| {
                t.decoration
                    .map(|z| (t.image.clone(), z, t.x, t.y, t.overhang))
            })
            .collect();
        let tab = (
            "assets/frame/main/decor-infotab.png".to_owned(),
            30,
            TileAxis::End { offset: 0 },
            TileAxis::Centre,
            true,
        );
        let logo = (
            "assets/frame/main/decor-sclogo.png".to_owned(),
            4,
            TileAxis::Start { offset: 0 },
            TileAxis::Start { offset: 1 },
            true,
        );
        assert_eq!(got, vec![tab, logo]);
    }

    /// Plan SK0-L1: the views' insets are counted from the tile laying the
    /// frame's nine-piece set (StealthX: `MainBackground` at X=112, its
    /// `ListBackground` 24/23 in, the views 2/2 in that), else from the
    /// root less the rows' outsets (Symbiosis: `XfireView` 5/18, rows
    /// `IndentRight="13"`); a reference placement reads none.
    #[test]
    fn views_insets_are_counted_from_the_frame() {
        let nine = parse(
            "Components/MainWindow.xml",
            br#"<Components><Component Name="Bg"><Images>
<Image Type="TopLeft">a</Image><Image Type="Top">a</Image><Image Type="TopRight">a</Image>
<Image Type="Left">a</Image><Image Type="Middle">a</Image><Image Type="Right">a</Image>
<Image Type="BottomLeft">a</Image><Image Type="Bottom">a</Image><Image Type="BottomRight">a</Image>
</Images></Component></Components>"#,
        )
        .unwrap();
        let stealth = parse(
            "MainWindow.xml",
            br#"<Tile Name="W" ResizeX="100" ResizeY="100" Component="MainWindow">
<Tile Name="MainBackground" X="112" Y="0" ResizeX="100" ResizeY="100" Component="Bg">
<Tile Name="List" IndentLeft="24" IndentRight="23" ResizeX="100" ResizeY="100" Component="List">
<Tile Name="TabbedControl" X="2" Y="2" IndentRight="2" ResizeX="100" ResizeY="100" Component="TabbedControl"/>
</Tile></Tile></Tile>"#,
        )
        .unwrap();
        assert_eq!(
            views_insets(&stealth, std::slice::from_ref(&nine)),
            Some((26, 25))
        );
        let symbiosis = parse(
            "MainWindow.xml",
            br#"<Tile Name="MainWindow" Component="MainWindow">
<Tile Name="TopRow" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="BottomRow" JustY="Bottom" ResizeX="100" IndentRight="13" Component="Row"/>
<Tile Name="XfireView" Y="Tabs.Bottom+1" ResizeX="100" ResizeY="100" IndentRight="18" IndentLeft="5" Component="XfireView"/>
</Tile>"#,
        )
        .unwrap();
        assert_eq!(
            views_insets(&symbiosis, std::slice::from_ref(&nine)),
            Some((5, 5))
        );
        let classic = parse(
            "MainWindow.xml",
            br#"<Tile Name="W" Component="MainWindow">
<Tile Name="Inner" X="Edge.RIGHT" ResizeX="100" ResizeY="100" Component="Inner">
<Tile Name="TabbedControl" X="7" IndentRight="6" ResizeX="100" ResizeY="100" Component="TabbedControl"/>
</Tile></Tile>"#,
        )
        .unwrap();
        assert_eq!(views_insets(&classic, &[nine]), None);
    }
}
