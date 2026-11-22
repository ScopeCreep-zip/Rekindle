//! A window frame laid as a stack of full-width rows (plan SK0 S1): Total
//! Chaos's `mainwindow.xml` lays `MainTitleBar`, `AdBar`, `MenuBar`,
//! `Content` and `BottomBar` under the root, each `ResizeX="100"` and
//! `Y="<previous>.BOTTOM"`, each component's normal set typed `left`,
//! `middle`, `right` (and the title's `center`, the T.C plaque). The row
//! that resizes down (`ResizeY="100"`) is the content row: its sides
//! repeat down the window and its middle is the content ground. A row's
//! left and right lie at its ends, its middle repeats between them at its
//! source size, and its centre piece is laid centred on the whole window
//! as an overhang (it sits over the title row's repeat, Xfire's
//! `JustX="CENTER"`). The frame rectangle is the rows' box and the outsets
//! are zero here: the rows' `OutdentLeft="1"` and a last row reaching past
//! the window box (Total Chaos's `BottomBar`, 66 px under a `Content`
//! `IndentBottom="61"`) are placed by the tree, which is handed the rows'
//! names and lays the frame rectangle where the pack places them (plan SK2
//! lead decision 14). Read only when no nine-piece shape is complete
//! ([`super::tiles::pieces`]).
//!
//! The content rectangle is the pack's own content region (plan SK0-L1):
//! the content row's views tile (`TabbedControl`, `ResizeX`/`ResizeY="100"`)
//! sits `X` px from the left and `IndentRight` px from the right, inside the
//! row's side pieces (Total Chaos: 50 px in 59 px sides), so the sides are
//! laid only to that edge (their inner columns lay under the pack's views,
//! never shown).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, SkinWindow, TileAxis, WindowLayout};

use super::super::tree::Node;
use super::super::views;
use super::super::ImportError;
use super::descendants;
use crate::shape::{self, u16_of};

/// A window layout and the tile images it names.
type Built = (WindowLayout, BTreeMap<String, Vec<u8>>);

/// One row's pieces: left, middle, right, centre.
type Row = [Option<RgbaImage>; 4];

/// A stacked row: whether it is the content row, its pieces, and its
/// tile in the layout (for the views and ad tiles under it).
struct Stacked<'a> {
    resizes: bool,
    row: Row,
    tile: &'a Node,
}

/// A whole-pixel attribute, absent as 0.
fn px(n: &Node, key: &str) -> u32 {
    n.attr_ci(key)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// The content row's views tile insets (left `X`, right `IndentRight`).
fn views_insets(content: &Node) -> Option<(u32, u32)> {
    content
        .children_named("Tile")
        .find(|t| t.attr_ci("ResizeX") == Some("100") && t.attr_ci("ResizeY") == Some("100"))
        .map(|t| (px(t, "X"), px(t, "IndentRight")))
}

const TYPES: [&str; 4] = ["left", "middle", "right", "center"];
const NAMES: [&str; 4] = ["left", "middle", "right", "centre"];

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `window`'s layout from the rows `layout` stacks and the rows' tile
/// names, or `None` when it is not such a stack; `piece(bitmap, shift
/// colour)` decodes a piece.
///
/// # Errors
/// What `piece` returns, or a piece that does not encode.
pub fn layout(
    window: SkinWindow,
    layout: &Node,
    components: &[Node],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
) -> Result<Option<(Built, Vec<String>)>, ImportError> {
    let by_name: BTreeMap<String, &Node> = components
        .iter()
        .flat_map(|n| descendants(n, "Component"))
        .filter_map(|c| Some((c.attr_ci("Name")?.to_ascii_lowercase(), c)))
        .collect();
    let mut rows: Vec<Stacked> = Vec::new();
    for tile in layout.children_named("Tile") {
        if tile.attr_ci("ResizeX") != Some("100") {
            continue;
        }
        let Some(c) = tile
            .attr_ci("Component")
            .and_then(|n| by_name.get(&n.to_ascii_lowercase()))
        else {
            continue;
        };
        let Some(set) = c
            .children_named("Images")
            .find(|i| i.attr_ci("State").is_none_or(|s| eq(s, "Normal")))
        else {
            continue;
        };
        let shift = set.attr_ci("Color");
        let mut row: Row = [None, None, None, None];
        for (slot, ty) in row.iter_mut().zip(TYPES) {
            if let Some(img) = set
                .children_named("Image")
                .find(|i| i.attr_ci("Type").is_some_and(|t| eq(t, ty)))
            {
                *slot = Some(piece(&img.text, shift)?);
            }
        }
        if row[0].is_some() && row[1].is_some() && row[2].is_some() {
            rows.push(Stacked {
                resizes: tile.attr_ci("ResizeY") == Some("100"),
                row,
                tile,
            });
        }
    }
    let Some(content_row) = rows.iter().position(|r| r.resizes) else {
        return Ok(None);
    };
    if rows.len() < 2 || rows.iter().filter(|r| r.resizes).count() != 1 {
        return Ok(None);
    }
    // The content row's sides end where its views begin.
    if let Some(insets) = views_insets(rows[content_row].tile) {
        if let [Some(l), _, Some(r), _] = &mut rows[content_row].row {
            views::to_views_edge(l, r, insets);
        }
    }
    let names = rows
        .iter()
        .filter_map(|r| r.tile.attr_ci("Name").or(r.tile.attr_ci("Component")))
        .map(|n| n.trim().to_owned())
        .collect();
    build(window, &rows, content_row).map(|built| Some((built, names)))
}

/// The frame tiles [`layout`] lays for `count` rows, by package path: each
/// row's `left`, `middle`, `right` and `centre` (plan SK2 lead decision 27:
/// the component tree draws each row once, in its own box, so under the
/// tree none of them is laid again at the window's edges).
#[must_use]
pub fn row_paths(window: SkinWindow, count: usize) -> Vec<String> {
    (0..count)
        .flat_map(|i| NAMES.map(|k| shape::tile_path(window, &format!("row{i}-{k}"))))
        .collect()
}

fn height(row: &Row) -> u32 {
    row[1].as_ref().map_or(0, RgbaImage::height)
}

fn width(img: Option<&RgbaImage>) -> u32 {
    img.map_or(0, RgbaImage::width)
}

fn build(window: SkinWindow, rows: &[Stacked], content_row: usize) -> Result<Built, ImportError> {
    let file = shape::tile_path(window, "frame");
    let top: u32 = rows[..content_row].iter().map(|r| height(&r.row)).sum();
    let bottom: u32 = rows[content_row + 1..].iter().map(|r| height(&r.row)).sum();
    let mut tiles = Vec::new();
    let mut files = BTreeMap::new();
    let (mut above, mut below) = (0u32, bottom);
    for (i, row) in rows.iter().map(|r| &r.row).enumerate() {
        let (lw, rw) = (width(row[0].as_ref()), width(row[2].as_ref()));
        let y = match i.cmp(&content_row) {
            std::cmp::Ordering::Less => {
                let y = TileAxis::Start {
                    offset: u16_of(&file, above)?,
                };
                above += height(row);
                y
            }
            std::cmp::Ordering::Equal => TileAxis::Repeat {
                start: u16_of(&file, top)?,
                end: u16_of(&file, bottom)?,
            },
            std::cmp::Ordering::Greater => {
                below -= height(row);
                TileAxis::End {
                    offset: u16_of(&file, below)?,
                }
            }
        };
        let middle = TileAxis::Repeat {
            start: u16_of(&file, lw)?,
            end: u16_of(&file, rw)?,
        };
        let xs = [
            TileAxis::Start { offset: 0 },
            middle,
            TileAxis::End { offset: 0 },
            TileAxis::Centre,
        ];
        for (k, (img, x)) in row.iter().zip(xs).enumerate() {
            // The content row's middle is the content ground.
            let Some(img) = img.as_ref().filter(|_| !(i == content_row && k == 1)) else {
                continue;
            };
            let name = format!("row{i}-{}", NAMES[k]);
            if let Some((t, png)) = shape::tile(window, &name, img, (x, y), k == 3)? {
                files.insert(t.image.clone(), png);
                tiles.push(t);
            }
        }
    }
    let content_sides = &rows[content_row].row;
    let content = Insets {
        top: u16_of(&file, top)?,
        right: u16_of(&file, width(content_sides[2].as_ref()))?,
        bottom: u16_of(&file, bottom)?,
        left: u16_of(&file, width(content_sides[0].as_ref()))?,
    };
    let min_width = rows
        .iter()
        .map(|r| &r.row)
        .map(|r| (width(r[0].as_ref()) + width(r[2].as_ref())).max(width(r[3].as_ref())))
        .max()
        .unwrap_or(0);
    let zero = Insets {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    };
    Ok((
        WindowLayout {
            outsets: zero,
            content,
            min_width: u16_of(&file, min_width)?,
            min_height: u16_of(&file, top + bottom)?,
            tiles,
            tree: None,
            title: None,
            popups: std::collections::BTreeMap::new(),
        },
        files,
    ))
}
