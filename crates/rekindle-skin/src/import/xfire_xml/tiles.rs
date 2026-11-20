//! The window frame of a late-layout tree without Symbiosis's
//! `Components/ChatWindow.xml` nine-piece set (plan E7.7.3 item 1; the
//! fan skins of the research archive, `.claude/plans/skins/raw/
//! xfire-online/ia_XFireSkins/`): the chat window's layout file (the
//! `<Include>` under `Skin.xml`'s `<ChatSkin>`) lays tiles, each naming a
//! component, and the frame is read from those tiles' components in one
//! of four shapes, the first that is complete winning. The shapes that name
//! the window's own frame tiles come first: a nine-piece set found on any
//! laid tile can be a panel inside the window (Space Cowboy's `ListViewBG`
//! at 26,181 is the buddy list's ground, not the window frame).
//!
//! 1. [`Shape::Named`]: tiles named for the nine positions (`TopLeft`,
//!    `Top`, `TopRight`, `LeftEdge`, `Middle`, `RightEdge`, `BottomLeft`,
//!    `Bottom`, `BottomRight`), each component's normal set holding the
//!    one image of that piece (Onyx, Space Cowboy).
//! 2. [`Shape::Sides`]: tiles `TopRow` and `BottomRow` whose sets hold
//!    `left`/`middle`/`right` (or the row's own nine-piece types), and `LeftSide`/`RightSide` (and an optional
//!    `Background` centre) of one image each (Symbiosis's
//!    `MainWindow.xml` "Window Frame" block).
//! 3. [`Shape::Nine`]: one component whose normal set has all nine piece
//!    types, Symbiosis's rule on the chat window's own components
//!    (StealthX's `MainBackground`).
//! 4. [`Shape::Rows`]: the first set with `topleft`/`top`/`topright`, the
//!    first with `bottomleft`/`bottom`/`bottomright`, and the first
//!    vertically resizing tile (`ResizeY="100"`, the only tiles Xfire
//!    stretches down the window) whose set has `left` and `right`
//!    (its `middle`, if any, the centre): Classic v3's rule with the
//!    components found through the layout (xSLIMFIRE).
//!
//! A shape without a centre piece has a transparent middle, as the INI era
//! and Classic v3 do. Names and types match ASCII-case-insensitively.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::Insets;

use super::super::tree::Node;
use super::super::ImportError;
use super::{bitmap, descendants, nine_piece_in};

/// Which rule the frame was read by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Nine,
    Rows,
    Named,
    Sides,
}

impl Shape {
    /// The rule, for the `NOTICE`.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Nine => "one component's nine-piece set",
            Self::Rows => {
                "the top and bottom rows' typed pieces and a vertically resizing tile's sides"
            }
            Self::Named => "the tiles named TopLeft … BottomRight, one piece each",
            Self::Sides => "the tiles TopRow, LeftSide, RightSide and BottomRow",
        }
    }
}

/// The tile names of [`Shape::Named`], in [`bitmap::NINE`] order.
const NAMED: [&[&str]; 9] = [
    &["TopLeft"],
    &["Top"],
    &["TopRight"],
    &["LeftEdge", "Left"],
    &["Middle"],
    &["RightEdge", "Right"],
    &["BottomLeft"],
    &["Bottom"],
    &["BottomRight"],
];

/// (bitmap name, colour name) of one piece.
type Piece = (String, Option<String>);

/// A tile of the layout with its component's normal image set.
struct Laid<'a> {
    tile: &'a Node,
    set: &'a Node,
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The component's first `<Images>` with no state or `State="Normal"`.
fn normal_set(component: &Node) -> Option<&Node> {
    component
        .children_named("Images")
        .find(|i| i.attr_ci("State").is_none_or(|s| eq(s, "Normal")))
}

fn typed(set: &Node, ty: &str) -> Option<Piece> {
    set.children_named("Image")
        .find(|i| i.attr_ci("Type").is_some_and(|t| eq(t, ty)))
        .map(|i| (i.text.clone(), set.attr_ci("Color").map(str::to_owned)))
}

fn row(laid: &[Laid<'_>], types: [&str; 3]) -> Option<[Piece; 3]> {
    laid.iter().find_map(|l| {
        let [a, b, c] = types.map(|t| typed(l.set, t));
        Some([a?, b?, c?])
    })
}

fn rows(laid: &[Laid<'_>]) -> Option<[Option<Piece>; 9]> {
    let top = row(laid, ["topleft", "top", "topright"])?;
    let bottom = row(laid, ["bottomleft", "bottom", "bottomright"])?;
    let (left, centre, right) = laid
        .iter()
        .filter(|tile| tile.tile.attr_ci("ResizeY") == Some("100"))
        .find_map(|tile| {
            Some((
                typed(tile.set, "left")?,
                typed(tile.set, "middle"),
                typed(tile.set, "right")?,
            ))
        })?;
    let [top_left, top_mid, top_right] = top.map(Some);
    let [bottom_left, bottom_mid, bottom_right] = bottom.map(Some);
    Some([
        top_left,
        top_mid,
        top_right,
        Some(left),
        centre,
        Some(right),
        bottom_left,
        bottom_mid,
        bottom_right,
    ])
}

fn tile_named<'a>(laid: &'a [Laid<'a>], names: &[&str]) -> Option<&'a Laid<'a>> {
    laid.iter().find(|l| {
        l.tile
            .attr_ci("Name")
            .is_some_and(|n| names.iter().any(|m| eq(m, n)))
    })
}

/// The one image of `l`'s set; `None` when it holds more or none.
fn single(l: &Laid<'_>) -> Option<Piece> {
    let mut images = l.set.children_named("Image");
    let first = images.next()?;
    images.next().is_none().then(|| {
        (
            first.text.clone(),
            l.set.attr_ci("Color").map(str::to_owned),
        )
    })
}

fn sides(laid: &[Laid<'_>]) -> Option<[Option<Piece>; 9]> {
    // A row's set types its pieces `left`/`middle`/`right` or by their
    // nine-piece position: Symbiosis's `MainTopRow` uses the first,
    // its `MainBottomRow` `BottomLeft`/`Bottom`/`BottomRight`.
    let row = |name: &str, own: [&str; 3]| {
        let l = tile_named(laid, &[name])?;
        [["left", "middle", "right"], own]
            .into_iter()
            .find_map(|types| {
                let [a, b, c] = types.map(|t| typed(l.set, t));
                Some([a?, b?, c?])
            })
    };
    let side = |name: &str| single(tile_named(laid, &[name])?);
    let [top_left, top, top_right] = row("TopRow", ["topleft", "top", "topright"])?.map(Some);
    let [bottom_left, bottom, bottom_right] =
        row("BottomRow", ["bottomleft", "bottom", "bottomright"])?.map(Some);
    Some([
        top_left,
        top,
        top_right,
        Some(side("LeftSide")?),
        side("Background"),
        Some(side("RightSide")?),
        bottom_left,
        bottom,
        bottom_right,
    ])
}

fn named(laid: &[Laid<'_>]) -> Option<[Option<Piece>; 9]> {
    let one = |names: &[&str]| single(tile_named(laid, names)?);
    let pieces = NAMED.map(one);
    pieces
        .iter()
        .enumerate()
        .all(|(i, p)| i == 4 || p.is_some())
        .then_some(pieces)
}

/// The frame laid by `layout` (the chat window's layout file) from
/// `components`, each piece decoded by `piece(bitmap name, shift colour)`;
/// `None` when no shape is complete.
///
/// # Errors
/// What `piece` returns, or pieces that do not compose.
pub fn frame(
    layout: &Node,
    components: &[Node],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
) -> Result<Option<(RgbaImage, Insets, Shape, [RgbaImage; 9])>, ImportError> {
    let Some((pieces, shape)) = pieces(layout, components, piece)? else {
        return Ok(None);
    };
    let (img, insets) = bitmap::compose("window layout", &pieces)?;
    Ok(Some((img, insets, shape, pieces)))
}

/// The nine pieces (in [`bitmap::NINE`] order) of the frame `layout`
/// lays, as [`frame`] reads them, uncomposed: a window's frame tiles
/// (plan SK0 S1); `None` when no shape is complete.
///
/// # Errors
/// What `piece` returns.
pub fn pieces(
    layout: &Node,
    components: &[Node],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
) -> Result<Option<([RgbaImage; 9], Shape)>, ImportError> {
    let mut by_name: BTreeMap<String, &Node> = BTreeMap::new();
    for c in components.iter().flat_map(|n| descendants(n, "Component")) {
        if let Some(name) = c.attr_ci("Name") {
            by_name.entry(name.to_ascii_lowercase()).or_insert(c);
        }
    }
    let laid: Vec<Laid<'_>> = descendants(layout, "Tile")
        .into_iter()
        .filter_map(|tile| {
            let c = by_name.get(&tile.attr_ci("Component")?.to_ascii_lowercase())?;
            Some(Laid {
                tile,
                set: normal_set(c)?,
            })
        })
        .collect();
    let nine = laid
        .iter()
        .find_map(|l| nine_piece_in([l.set]).map(|p| p.map(Some)));
    let found = named(&laid)
        .map(|p| (p, Shape::Named))
        .or_else(|| sides(&laid).map(|p| (p, Shape::Sides)))
        .or_else(|| nine.map(|p| (p, Shape::Nine)))
        .or_else(|| rows(&laid).map(|p| (p, Shape::Rows)));
    let Some((set, shape)) = found else {
        return Ok(None);
    };
    let mut pieces: Vec<RgbaImage> = Vec::with_capacity(9);
    for p in &set {
        pieces.push(match p {
            Some((name, colour)) => piece(name, colour.as_deref())?,
            None => RgbaImage::new(1, 1),
        });
    }
    let pieces: [RgbaImage; 9] = pieces.try_into().map_err(|_| ImportError::MissingFile {
        file: "frame pieces".to_owned(),
    })?;
    Ok(Some((pieces, shape)))
}

#[cfg(test)]
mod tests {
    use super::super::super::tree::parse;
    use super::*;

    /// Symbiosis's "Window Frame" block (`MainWindow.xml`): the top row
    /// typed `Left`/`Middle`/`Right`, the bottom row by position.
    #[test]
    fn a_bottom_row_typed_by_position_is_the_sides_shape() {
        let layout = parse(
            "MainWindow.xml",
            br#"<Tile Name="MainWindow" Component="MainWindow">
<Tile Name="TopRow" Component="MainTopRow"/><Tile Name="LeftSide" Component="MainLeftSide"/>
<Tile Name="RightSide" Component="MainRightSide"/><Tile Name="BottomRow" Component="MainBottomRow"/></Tile>"#,
        )
        .unwrap();
        let components = parse(
            "Components/MainWindow.xml",
            br#"<Components>
<Component Name="MainTopRow"><Images State="Normal"><Image Type="Left">tl</Image><Image Type="Middle">t</Image><Image Type="Right">tr</Image></Images></Component>
<Component Name="MainLeftSide"><Images State="Normal"><Image Type="Middle">l</Image></Images></Component>
<Component Name="MainRightSide"><Images State="Normal"><Image Type="Right">r</Image></Images></Component>
<Component Name="MainBottomRow"><Images State="Normal"><Image Type="BottomLeft">bl</Image><Image Type="Bottom">b</Image><Image Type="BottomRight">br</Image></Images></Component>
</Components>"#,
        )
        .unwrap();
        let names = std::cell::RefCell::new(Vec::new());
        let got = pieces(&layout, &[components], |name, _| {
            names.borrow_mut().push(name.to_owned());
            Ok(RgbaImage::new(2, 2))
        })
        .unwrap()
        .map(|(_, shape)| shape);
        assert_eq!(got, Some(Shape::Sides));
        assert_eq!(
            names.into_inner(),
            ["tl", "t", "tr", "l", "r", "bl", "b", "br"]
        );
    }
}
