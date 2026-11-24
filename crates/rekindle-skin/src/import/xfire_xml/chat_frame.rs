//! The chat window's frame fitted to the pack's own chat content region
//! (plan SK0-L1, lead decision: "the content box is the pack's content
//! region exactly", read for the chat window as [`super::decor`] reads it
//! for the buddy list; the chat layout import of SK17-2).
//!
//! A fan pack's chat layout (`chatwindow.xml`, the `<Include>` under
//! `Skin.xml`'s `<ChatSkin>`) cuts its frame art into nine tiles at any
//! split: Onyx at the art's middle (corners 83×87, `Middle` at 83,87),
//! xSLIMFIRE likewise (125×165). The tiles' sizes are not the frame's
//! border; the pack lays its chat area and edit box over the inner part
//! of the corner and edge tiles, `X`/`IndentRight` px from the sides,
//! `Y` px from the top (`ChatArea`) and `IndentBottom` px from the bottom
//! (`ChatEdit`, its parents' indents summed: Space Cowboy's sits in
//! `ListViewBG2`). The side and row pieces are laid only to that region's
//! edge ([`views::to_views_edge`], [`views::to_views_rows`]), so the
//! content rectangle is the pack's; a side narrower than the region's
//! inset is widened with the pack's centre fill ([`views::fill_band`]),
//! and a row likewise ([`fill_rows`]: Space Cowboy's 2 px rails under a
//! chat area 53 px below the cut top and an edit box 11 px above the cut
//! bottom).
//!
//! Before that, a frame's fully transparent outer columns and rows are not
//! window: Space Cowboy's edge pieces hold a 2 px line inside 18–55 px of
//! transparent margin, and laid at the window's edges they left uncovered
//! bands inside the window. Each side's margin common to its three pieces
//! is cut, and the region's insets are counted from the cut edge.
//!
//! The top inset is `ChatArea`'s `Y` exactly (lead decision, SK0-L1: the
//! content box is the pack's content region), so the pack's own title and
//! menu art above the chat area shows in the frame's top band until plan
//! SK17-2 imports the chat title strip.
//!
//! The window's minimum size is the pack's declared one: the `MinWidth` and
//! `MinHeight` of the component the layout's root names (Onyx's
//! `ChatWindow` 253×280, Space Cowboy's 344×350, xSLIMFIRE's
//! `ChatMainWindow` 255×300). Trimmed pieces alone say nothing of it (Space
//! Cowboy's 2 px rails gave an 18×4 window). Without a declared minimum the
//! window holds its insets and one copy of the centre fill. Every chat
//! frame source reads it so (Symbiosis's `ChatMainWindow` 332×290, the
//! classic v3 layout's 300×270), not only the nine tiles fitted here.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinWindow, TileAxis, WindowLayout};

use super::super::tree::Node;
use super::super::views;
use super::super::xfire_ini::lookup;
use super::super::{ImportError, Tree};
use super::decor::num;
use super::{
    bitmap, bitmaps, descendants, nine_piece, piece, tiles, v3, window_layout, xml, ColourRow,
    Layout, FRAME_SOURCE,
};
use crate::shape::{self, u16_of};

/// What [`fit`] read from the pack for [`window`]: the content region's
/// insets (left, top, right, bottom) from the cut frame edges, to widen
/// the sides and rows to, and the pack's declared minimum (width, height).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Fitted {
    pub region: Option<[u32; 4]>,
    pub declared: Option<(u16, u16)>,
}

/// The `MinWidth`/`MinHeight` of the component `layout`'s root names,
/// `None` unless both are pixel counts.
pub(super) fn declared_min(layout: &Node, components: &[Node]) -> Option<(u16, u16)> {
    let name = layout.attr_ci("Component")?;
    let comp = components
        .iter()
        .flat_map(|n| descendants(n, "Component"))
        .find(|c| {
            c.attr_ci("Name")
                .is_some_and(|m| m.eq_ignore_ascii_case(name))
        })?;
    let limit = |k: &str| comp.attr_ci(k)?.trim().parse::<u16>().ok();
    Some((limit("MinWidth")?, limit("MinHeight")?))
}

/// The tile named `name` under `n` and the tiles above it.
fn path_to<'a>(n: &'a Node, name: &str, path: &mut Vec<&'a Node>) -> bool {
    for t in n.children_named("Tile") {
        path.push(t);
        let found = t
            .attr_ci("Name")
            .is_some_and(|m| m.eq_ignore_ascii_case(name));
        if found || path_to(t, name, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// `attrs` summed down the path to the tile `name`; `None` without the
/// tile, when a placement on the way is not a pixel count or when a tile
/// is justified to another edge than `just`'s allowed value.
fn summed(layout: &Node, name: &str, attrs: &[&str], just: (&str, &str)) -> Option<u32> {
    let mut path = Vec::new();
    if !path_to(layout, name, &mut path) {
        return None;
    }
    let mut total = 0u32;
    for t in path {
        if t.attr_ci(just.0)
            .is_some_and(|j| !j.eq_ignore_ascii_case(just.1))
        {
            return None;
        }
        for a in attrs {
            total += u32::from(num(t, a)?);
        }
    }
    Some(total)
}

/// The pack's chat content region (left, top, right, bottom) inside the
/// layout's window: `ChatArea`'s left, top and right, `ChatEdit`'s bottom.
fn region(layout: &Node) -> Option<[u32; 4]> {
    let left = summed(layout, "ChatArea", &["X", "IndentLeft"], ("JustX", "LEFT"))?;
    let top = summed(layout, "ChatArea", &["Y", "IndentTop"], ("JustY", "TOP"))?;
    let right = summed(layout, "ChatArea", &["IndentRight"], ("JustX", "LEFT"))?;
    let bottom = summed(layout, "ChatEdit", &["IndentBottom"], ("JustY", "TOP"))?;
    Some([left, top, right, bottom])
}

fn clear(img: &RgbaImage, x: u32, y: u32) -> bool {
    img.get_pixel(x, y)[3] == 0
}

/// How many of `img`'s columns from one side (`from_left`) are wholly
/// transparent, leaving at least one.
fn clear_columns(img: &RgbaImage, from_left: bool) -> u32 {
    let w = img.width();
    let col = |i: u32| if from_left { i } else { w - 1 - i };
    (0..w.saturating_sub(1))
        .take_while(|&i| (0..img.height()).all(|y| clear(img, col(i), y)))
        .count()
        .try_into()
        .unwrap_or(0)
}

/// How many of `img`'s rows from one side (`from_top`) are wholly
/// transparent, leaving at least one.
fn clear_rows(img: &RgbaImage, from_top: bool) -> u32 {
    let h = img.height();
    let row = |i: u32| if from_top { i } else { h - 1 - i };
    (0..h.saturating_sub(1))
        .take_while(|&i| (0..img.width()).all(|x| clear(img, x, row(i))))
        .count()
        .try_into()
        .unwrap_or(0)
}

/// Cuts each side's transparent margin common to its three pieces;
/// returns the cuts (left, top, right, bottom).
pub(super) fn trim(pieces: &mut [RgbaImage; 9]) -> [u32; 4] {
    let min = |idx: [usize; 3], f: &dyn Fn(&RgbaImage) -> u32| {
        idx.iter().map(|&i| f(&pieces[i])).min().unwrap_or(0)
    };
    let left = min([0, 3, 6], &|p| clear_columns(p, true));
    let right = min([2, 5, 8], &|p| clear_columns(p, false));
    let top = min([0, 1, 2], &|p| clear_rows(p, true));
    let bottom = min([6, 7, 8], &|p| clear_rows(p, false));
    for i in [0, 3, 6] {
        pieces[i] = views::last_columns(&pieces[i], pieces[i].width() - left);
    }
    for i in [2, 5, 8] {
        pieces[i] = views::first_columns(&pieces[i], pieces[i].width() - right);
    }
    for i in [0, 1, 2] {
        pieces[i] = views::last_rows(&pieces[i], pieces[i].height() - top);
    }
    for i in [6, 7, 8] {
        pieces[i] = views::first_rows(&pieces[i], pieces[i].height() - bottom);
    }
    [left, top, right, bottom]
}

/// Fits the nine `pieces` the chat `layout` lays to the pack's chat
/// content region (module docs) and reads the pack's declared minimum
/// from `components`; the band is `None` when the layout names no numeric
/// region.
pub(super) fn fit(layout: &Node, components: &[Node], pieces: &mut [RgbaImage; 9]) -> Fitted {
    let cut = trim(pieces);
    Fitted {
        region: fit_region(layout, pieces, cut),
        declared: declared_min(layout, components),
    }
}

fn fit_region(layout: &Node, pieces: &mut [RgbaImage; 9], cut: [u32; 4]) -> Option<[u32; 4]> {
    let [left, top, right, bottom] = region(layout)?;
    let (left, right) = (left.saturating_sub(cut[0]), right.saturating_sub(cut[2]));
    let (top, bottom) = (top.saturating_sub(cut[1]), bottom.saturating_sub(cut[3]));
    {
        let [_, _, _, l, _, r, _, _, _] = &mut *pieces;
        views::to_views_edge(l, r, (left, right));
    }
    views::to_views_rows(pieces, (top, bottom));
    Some([left, top, right, bottom])
}

/// The frame and where it was read.
pub(super) type ReadFrame = (
    Option<(RgbaImage, rekindle_types::dto::skin::Insets)>,
    String,
    Option<([RgbaImage; 9], Fitted)>,
);

/// The chat window's frame from `tree`: the classic v3 rails, the nine
/// pieces of [`FRAME_SOURCE`] or the nine tiles the chat layout lays (the
/// latter fitted to the pack's content region, [`fit`]), each with the
/// pack's declared minimum, and where it was read.
///
/// # Errors
/// A missing or malformed file, or a piece that does not decode or compose.
pub(super) fn read_frame(
    tree: &Tree,
    (skin, theme): (&Node, &Node),
    colours: &[ColourRow],
    layout: Layout,
    components: &[Node],
) -> Result<ReadFrame, ImportError> {
    let bitmaps = bitmaps(theme);
    let decode = |name: &str, shift: Option<&str>| piece(tree, &bitmaps, colours, name, shift);
    let chat_layout = window_layout(skin, "ChatSkin").filter(|f| lookup(tree, f).is_ok());
    let declared = match chat_layout {
        Some(file) => declared_min(&xml(tree, file)?, components),
        None => None,
    };
    let unfitted = Fitted {
        region: None,
        declared,
    };
    if layout == Layout::ClassicV3 {
        let source = "xfirecomponents.xml (the chat window's outer edges' middles, \
                      9 px rails; no top or bottom row; transparent middle)";
        let read = v3::frame(&xml(tree, v3::SOURCE)?, decode)?;
        let (frame, pieces) = read.map(|(i, n, p)| ((i, n), (p, unfitted))).unzip();
        return Ok((frame, source.to_owned(), pieces));
    }
    let set = if lookup(tree, FRAME_SOURCE).is_ok() {
        nine_piece(&xml(tree, FRAME_SOURCE)?)
    } else {
        None
    };
    let Some(set) = set else {
        let Some(file) = chat_layout else {
            return Ok((None, String::new(), None));
        };
        let layout = xml(tree, file)?;
        return Ok(match tiles::pieces(&layout, components, decode)? {
            Some((mut pieces, shape, _)) => {
                let fitted = fit(&layout, components, &mut pieces);
                (
                    Some(bitmap::compose("window layout", &pieces)?),
                    format!("the chat window layout {file}: {}", shape.describe()),
                    Some((pieces, fitted)),
                )
            }
            None => (None, String::new(), None),
        });
    };
    let mut pieces: Vec<RgbaImage> = Vec::with_capacity(9);
    for (name, colour) in &set {
        pieces.push(decode(name, colour.as_deref())?);
    }
    let pieces: [RgbaImage; 9] = pieces.try_into().map_err(|_| ImportError::MissingFile {
        file: FRAME_SOURCE.to_owned(),
    })?;
    let frame = bitmap::compose(FRAME_SOURCE, &pieces)?;
    Ok((
        Some(frame),
        FRAME_SOURCE.to_owned(),
        Some((pieces, unfitted)),
    ))
}

/// Widens `built`'s content to the region's `insets` (top, bottom) where a
/// row is shorter than its inset, painting the band between the row and
/// the region with the pack's `centre` fill across the content's width, as
/// [`views::fill_band`] does the sides (same rule: the fill must be
/// present and wholly opaque, else no band and the row's height stays).
fn fill_rows(
    built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>),
    centre: &RgbaImage,
    (top, bottom): (u32, u32),
) -> Result<(), ImportError> {
    let present = centre.width() > 0
        && centre.height() > 0
        && centre.pixels().all(|p| p[3] >= shape::ALPHA_OPAQUE);
    if !present {
        return Ok(());
    }
    let file = shape::tile_path(SkinWindow::Chat, "frame");
    let c = built.0.content;
    let x = TileAxis::Repeat {
        start: c.left,
        end: c.right,
    };
    for (is_top, inset) in [(true, top), (false, bottom)] {
        let edge = if is_top { c.top } else { c.bottom };
        if inset <= u32::from(edge) {
            continue;
        }
        let img = RgbaImage::from_fn(centre.width(), inset - u32::from(edge), |x, y| {
            *centre.get_pixel(x, y % centre.height())
        });
        let (name, y) = if is_top {
            ("views-band-top", TileAxis::Start { offset: edge })
        } else {
            ("views-band-bottom", TileAxis::End { offset: edge })
        };
        let Some((mut t, png)) = shape::tile(SkinWindow::Chat, name, &img, (x, y), false)? else {
            continue;
        };
        t.fill = true;
        built.1.insert(t.image.clone(), png);
        built.0.tiles.push(t);
        let inset = u16_of(&file, inset)?;
        if is_top {
            built.0.content.top = inset;
        } else {
            built.0.content.bottom = inset;
        }
    }
    let c = built.0.content;
    built.0.min_height = built.0.min_height.max(c.top.saturating_add(c.bottom));
    Ok(())
}

/// The chat window's layout from its nine `pieces`, the rows and sides
/// widened to `fitted`'s region with the centre fill, at least `fitted`'s declared
/// minimum (module docs).
///
/// # Errors
/// Pieces too large for a tile, or that do not encode.
pub(super) fn window(
    pieces: &[RgbaImage; 9],
    fitted: Fitted,
) -> Result<(WindowLayout, BTreeMap<String, Vec<u8>>), ImportError> {
    let mut built = shape::from_nine(SkinWindow::Chat, pieces)?;
    if let Some([left, top, right, bottom]) = fitted.region {
        fill_rows(&mut built, &pieces[4], (top, bottom))?;
        views::fill_band(SkinWindow::Chat, &mut built, &pieces[4], (left, right))?;
    }
    let c = built.0.content;
    let (need_w, need_h) = fitted.declared.unwrap_or_else(|| {
        let fill = |inset: u32, len: u32| u16::try_from(inset + len).unwrap_or(u16::MAX);
        (
            fill(u32::from(c.left) + u32::from(c.right), pieces[4].width()),
            fill(u32::from(c.top) + u32::from(c.bottom), pieces[4].height()),
        )
    });
    let layout = &mut built.0;
    layout.min_width = layout.min_width.max(need_w);
    layout.min_height = layout.min_height.max(need_h);
    Ok(built)
}

#[cfg(test)]
#[path = "chat_frame_tests.rs"]
mod tests;
