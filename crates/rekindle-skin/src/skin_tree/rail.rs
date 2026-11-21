//! The side rail's ground drawn by the frame (plan SK2 lead decision 15:
//! "StealthX's rail must be in the silhouette … its `MENU_*` ground art is
//! laid as decoration in S1"). The rail (`MainMenuPopup`, laid by
//! `with_rail`) holds a tile stretched over it whose component is a
//! nine-piece set (`MainMenuBackground` `Component="MenuBackground"`,
//! `popups.xml:3`; `MENU_TOP_LEFT` … `MENU_BOTTOM_RIGHT`,
//! `components.xml:26-40`). Xfire tiles a nine-piece set over its box
//! (corners once, edges and centre repeated at source size), so the art is
//! laid as three decoration tiles over that node's box: the top row once,
//! the middle band repeated down, the bottom row once, each placed in the
//! whole window (`overhang`) because the rail lies left of the pack's
//! `MainBackground` frame (`mainwindow.xml:2` `X="112"`). They are frame
//! tiles, so they draw beneath every tree node, the rail's own children
//! among them.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinWindow, TileAxis, WindowLayout};
use rekindle_types::dto::skin_tree::{NodeRole, SkinTree};

use super::import::bad;
use crate::import::tree::Node;
use crate::import::xfire_xml::nine_piece_in;
use crate::import::ImportError;
use crate::shape;

/// The window sizes the rail's box is read at: it must be the same box
/// (fixed across, a fixed inset from the top and the bottom) at both.
const SIZES: [i32; 2] = [4096, 3996];

/// Every tile under `n`, depth first.
fn tiles<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    for t in n.children_named("Tile") {
        out.push(t);
        tiles(t, out);
    }
}

/// `node`'s box as (x, width, top inset, bottom inset) in the window, the
/// same at both [`SIZES`]; `None` when it moves or changes size across.
fn fixed_box(tree: &SkinTree, id: &str) -> Option<(u16, u16, u16, u16)> {
    let at = |m: i32| {
        tree.boxes(m, m)
            .into_iter()
            .find(|(n, _)| n.id == id)
            .map(|(_, b)| [b[0], b[2], b[1], m - b[1] - b[3]])
    };
    let a = at(SIZES[0])?;
    (at(SIZES[1])? == a).then_some(())?;
    let [x, w, top, bottom] = a.map(|v| u16::try_from(v).ok());
    Some((x?, w?, top?, bottom?))
}

/// The row of `pieces` (left, middle, right) laid across `width`: the ends
/// once, the middle repeated between them at its source size, the last
/// copy cut; `rows` tall, each piece repeated down from the row's top.
fn compose(pieces: [&RgbaImage; 3], width: u32, rows: u32) -> Option<RgbaImage> {
    let [l, m, r] = pieces;
    let right_at = width.checked_sub(r.width())?;
    if l.width() > right_at || m.width() == 0 || [l, m, r].iter().any(|p| p.height() == 0) {
        return None;
    }
    Some(RgbaImage::from_fn(width, rows, |x, y| {
        if x < l.width() {
            *l.get_pixel(x, y % l.height())
        } else if x >= right_at {
            *r.get_pixel(x - right_at, y % r.height())
        } else {
            *m.get_pixel((x - l.width()) % m.width(), y % m.height())
        }
    }))
}

/// The least common multiple of `a` and `b` (the middle band's height,
/// after which its three repeated columns line up again).
fn lcm(a: u32, b: u32) -> u32 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

/// Lays the side rail's nine-piece ground of `tree` (its components named
/// in `by_name`, its tiles in `laid`) as decoration tiles of `built`, each
/// image decoded by `piece`; nothing when the tree has no side rail or the
/// rail holds no nine-piece tile.
///
/// # Errors
/// A rail ground whose box is not fixed across and inset from the top and
/// bottom, rows whose pieces differ in height, a box narrower than its
/// corners, or what `piece` returns.
pub(super) fn lay(
    window: SkinWindow,
    tree: &SkinTree,
    (laid, by_name): (&Node, &BTreeMap<String, &Node>),
    piece: &impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
    built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>),
) -> Result<(), ImportError> {
    let Some(rail) = tree.root.iter().find(|n| n.role == NodeRole::SideRail) else {
        return Ok(());
    };
    let Some(tile) = laid
        .children_named("Tile")
        .find(|t| t.attr_ci("Name").is_some_and(|n| n.trim() == rail.id))
    else {
        return Err(bad(format!("side rail {} is not in the layout", rail.id)));
    };
    let mut under = Vec::new();
    tiles(tile, &mut under);
    let Some((ground, set)) = under.into_iter().find_map(|t| {
        let c = by_name.get(&t.attr_ci("Component")?.to_ascii_lowercase())?;
        Some((
            t.attr_ci("Name")?.trim(),
            nine_piece_in(c.children_named("Images"))?,
        ))
    }) else {
        return Ok(());
    };
    let (x, width, top, bottom) = fixed_box(tree, ground)
        .ok_or_else(|| bad(format!("rail ground {ground} is not a fixed box")))?;
    let img: Vec<RgbaImage> = set
        .iter()
        .map(|(name, colour)| piece(name, colour.as_deref()))
        .collect::<Result<_, _>>()?;
    let row = |i: usize| -> Result<RgbaImage, ImportError> {
        let p = [&img[i], &img[i + 1], &img[i + 2]];
        let h = p[0].height();
        let even = p.iter().all(|q| q.height() == h);
        even.then(|| compose(p, u32::from(width), h))
            .flatten()
            .ok_or_else(|| bad(format!("rail ground {ground}: uneven or too wide row {i}")))
    };
    let band = img[3..6].iter().fold(1, |a, p| lcm(a, p.height().max(1)));
    let middle = compose([&img[3], &img[4], &img[5]], u32::from(width), band)
        .ok_or_else(|| bad(format!("rail ground {ground}: the middle band is too wide")))?;
    let (head, foot) = (row(0)?, row(6)?);
    let size = |i: &RgbaImage| u16::try_from(i.height()).map_err(|_| bad("rail art too tall"));
    let (head_h, foot_h) = (size(&head)?, size(&foot)?);
    let z = u8::try_from(rail.z.max(0)).unwrap_or(u8::MAX);
    let across = TileAxis::Start { offset: x };
    for (name, img, down) in [
        ("rail-top", &head, TileAxis::Start { offset: top }),
        (
            "rail-middle",
            &middle,
            TileAxis::Repeat {
                start: top + head_h,
                end: bottom + foot_h,
            },
        ),
        ("rail-bottom", &foot, TileAxis::End { offset: bottom }),
    ] {
        let Some((mut t, png)) = shape::tile(window, name, img, (across, down), true)? else {
            continue;
        };
        t.decoration = Some(z);
        built.1.insert(t.image.clone(), png);
        built.0.tiles.push(t);
    }
    Ok(())
}
