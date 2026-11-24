//! The style of a worded Communities clone's words (plan SK15): the clone
//! ([`super::place`]) copies its source tab's art, word and all (XGlow's
//! 65 x 18 `Downloads`, its "Downloads" drawn in at x 2, 7 px tall, in
//! `#5E90A8`). The pack's tab words are pictures, not text with a declared
//! font, so the Host measures the copied word at serve time from the
//! package's own art, as [`super::cover`] measures a copied icon: the same
//! package always serves the same style, and nothing is written into it
//! (rule N4, [`super::place`]).
//!
//! - The word's ink is what stands off its row's ground in the word's hue
//!   ([`ink`]): its box is the cover that hides the copied word. Art with
//!   no such word (a band taller than half the art: xslimfire's gloss)
//!   keeps the cover [`super::place`] gave and the shell's label style.
//! - Its weight is the word's stem width ([`stem`]): the commonest run of
//!   ink across a row of the word, its vertical strokes (a word's bars and
//!   bowls are a few long runs, its stems one per stroke per row). The
//!   packs' words are unantialiased GDI raster: XGlow's tab words have 1 px
//!   stems throughout, Tahoma Regular (400); xslimfire's have 2 px stems,
//!   Tahoma Bold (700). A run of 2 px or more is bold.
//! - Its size is the largest whose Tahoma cap height (OS/2 `sCapHeight`
//!   1489 of 2048 units, the same in Regular and Bold) rounds to no more
//!   than the ink's height: a tab word is capitalised (Friends, Servers,
//!   Downloads, Files), so its ink is its capitals, and the raster rounds
//!   the cap height to whole pixels; 7 px of ink is 10 px (cap 7.27).
//!   GDI drew the art with Tahoma's hinted device widths (its `hdmx` at 10
//!   ppem): XGlow's "Friends", "Servers" and "Downloads" are 34, 34 and 51
//!   px of ink, Regular's 33, 34 and 50 px of advance; xslimfire's
//!   "Friends" and "Servers" are 36 and 38, Bold's 37 and 39.
//! - Its colour is the brightest ink pixel (the word's core; its edges
//!   blend into the ground).
//! - It sits where the source word sits: centred when the word's margins
//!   in the art are even (within 2 px: xslimfire's "Files"), the words
//!   then laid across the node less its frame; otherwise from the word's
//!   ink start across the rest of the node (XGlow's, at x 2).
//! - The Host fits the words at the measured weight with Tahoma's exact
//!   advances ([`REGULAR`], [`BOLD_ADVANCES`], font units of its 2048 em;
//!   the shell lays text unhinted), summed and scaled once per size,
//!   against the text box with no margin: the size steps down until
//!   "Communities" fits. XGlow's Regular 10 px (11679 units) is 57.0 px of
//!   its 63 px from x 2; xslimfire's Bold 8 px (13516 units) is 52.8 px of
//!   its 53 px box (9 px, 59.4, does not). [`super::content::word_width`]'s per-letter
//!   round-up at 10 px (67 px for the word, 53.6 px scaled to 8) is a
//!   box that never cuts a label, and fitting with it stepped a size too
//!   small.

use std::collections::HashMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinError, WindowLayout};
use rekindle_types::dto::skin_tree::{
    LabelAlign, LabelStyle, NodeLabel, NodeRole, NodeSpan, SkinNode,
};
use rekindle_types::dto::skin_tree_role::ViewKey;

use crate::package::Package;

use super::cover::{each_root, picture_frame, seen, ICON_TOLERANCE};

/// Tahoma's cap height and em, font units (OS/2 table).
const CAP_UNITS: u32 = 1489;
const EM_UNITS: u32 = 2048;
/// CSS font weights of Tahoma's two faces.
pub(crate) const REGULAR_WEIGHT: u16 = 400;
pub(crate) const BOLD_WEIGHT: u16 = 700;
/// Tahoma Regular's advance widths, font units of [`EM_UNITS`], for ASCII
/// 32 (space) to 126 (`~`), in order (its `hmtx` through its `cmap`).
const REGULAR: [u32; 95] = [
    640, 680, 822, 1490, 1118, 2000, 1380, 432, 784, 784, 1118, 1490, 620, 744, 620, 783, 1118,
    1118, 1118, 1118, 1118, 1118, 1118, 1118, 1118, 1118, 724, 724, 1490, 1490, 1490, 970, 1862,
    1228, 1207, 1230, 1389, 1149, 1068, 1367, 1383, 764, 853, 1204, 1019, 1578, 1367, 1449, 1129,
    1449, 1271, 1141, 1196, 1343, 1222, 1847, 1189, 1180, 1145, 784, 783, 784, 1490, 1118, 1118,
    1075, 1132, 945, 1132, 1078, 652, 1132, 1142, 468, 577, 1020, 468, 1720, 1142, 1112, 1132,
    1132, 738, 914, 685, 1142, 1020, 1520, 1014, 1020, 910, 984, 783, 984, 1490,
];
/// Tahoma Bold's advance widths, as [`REGULAR`].
const BOLD_ADVANCES: [u32; 95] = [
    600, 702, 1002, 1676, 1304, 2455, 1600, 564, 930, 930, 1304, 1676, 640, 883, 640, 1182, 1304,
    1304, 1304, 1304, 1304, 1304, 1304, 1304, 1304, 1304, 744, 744, 1676, 1676, 1676, 1160, 1884,
    1402, 1405, 1367, 1551, 1260, 1190, 1526, 1565, 990, 1025, 1426, 1172, 1829, 1578, 1577, 1346,
    1577, 1487, 1297, 1254, 1513, 1382, 2105, 1402, 1373, 1275, 930, 1182, 930, 1676, 1304, 1118,
    1226, 1294, 1080, 1289, 1216, 783, 1289, 1311, 618, 743, 1234, 618, 1953, 1311, 1264, 1289,
    1289, 888, 1054, 851, 1311, 1185, 1822, 1238, 1179, 1077, 1276, 1304, 1276, 1676,
];
/// Margins this close in px read as a centred word.
const CENTRED: usize = 2;

/// A word's ink in its picture: `[left, top, width, height]` and the
/// colour of its brightest pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ink {
    pub(crate) bounds: [usize; 4],
    pub(crate) brightest: [u8; 3],
    pub(crate) centred: bool,
    /// The word's stem width, px ([`stem`]).
    pub(crate) stem: usize,
}

/// The commonest length of a horizontal run of `mask` (the word's ink,
/// row by row, inside its box), the shorter on a tie: a word's vertical
/// strokes cross every row they span, each as one run as wide as the
/// stem.
pub(crate) fn stem(mask: &[Vec<bool>]) -> usize {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for row in mask {
        let mut run = 0;
        for &on in row.iter().chain(std::iter::once(&false)) {
            if on {
                run += 1;
            } else if run > 0 {
                *counts.entry(run).or_default() += 1;
                run = 0;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(len, count)| (count, std::cmp::Reverse(len)))
        .map_or(1, |(len, _)| len)
}

/// The weight of a word whose stems are `stem` px: 2 px or more is bold.
pub(crate) fn weight_for(stem: usize) -> u16 {
    if stem >= 2 {
        BOLD_WEIGHT
    } else {
        REGULAR_WEIGHT
    }
}

/// Each row's ground: its commonest shown colour (a word is a few pixels
/// of its row; a tab's gloss and slanted end change down the art, and its
/// flanks can be the window outside the tab, XGlow's right end).
fn grounds(rows: &[Vec<[u8; 4]>]) -> Vec<[u8; 4]> {
    rows.iter()
        .map(|row| {
            let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
            for colour in row {
                *counts.entry(*colour).or_default() += 1;
            }
            counts
                .into_iter()
                .max_by_key(|&(colour, count)| (count, colour))
                .map_or([0; 4], |(colour, _)| colour)
        })
        .collect()
}

/// The hue sign of `v` across each channel pair, `0` where the pair is
/// within `margin`.
fn hue(v: [i16; 3], margin: i16) -> [i8; 3] {
    let sign = |a: i16, b: i16| match a - b {
        d if d > margin => 1,
        d if d < -margin => -1,
        _ => 0,
    };
    [sign(v[0], v[1]), sign(v[1], v[2]), sign(v[0], v[2])]
}

/// The word's ink in `image`, or `None` when it has none or more than a
/// band of half its height (a gloss or plate, not a word).
///
/// The word's colour is the brightest pixel standing more than
/// [`ICON_TOLERANCE`] off its row's ground; its ink is every such pixel of
/// the word's hue (each channel pair ordered as the word colour's, where
/// that pair differs by 16 or more: XGlow's blue word and its antialiased
/// edge against a grey plate, never the plate's grey rim or the window
/// outside). A grey word keeps every standing-off pixel.
pub(crate) fn ink(image: &RgbaImage) -> Option<Ink> {
    let width = usize::try_from(image.width()).ok()?;
    let height = usize::try_from(image.height()).ok()?;
    let rows: Vec<Vec<[u8; 4]>> = image
        .rows()
        .map(|r| r.map(|p| seen(p.0)).collect())
        .collect();
    let ground = grounds(&rows);
    let off = |p: [u8; 4], g: [u8; 4]| {
        p[3] > 0 && p.iter().zip(g).any(|(a, b)| a.abs_diff(b) > ICON_TOLERANCE)
    };
    let rgb = |p: [u8; 4]| [i16::from(p[0]), i16::from(p[1]), i16::from(p[2])];
    let brightest = rows
        .iter()
        .zip(&ground)
        .flat_map(|(row, &g)| row.iter().filter(move |&&p| off(p, g)))
        .max_by_key(|p| u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]))?;
    let key = hue(rgb(*brightest), 15);
    let mut found: Option<[usize; 4]> = None;
    let mut mask = vec![vec![false; width]; height];
    for (row, (pixels, &g)) in rows.iter().zip(&ground).enumerate() {
        for (col, &p) in pixels.iter().enumerate() {
            let [r, gr, b] = rgb(p);
            let d = [
                r - i16::from(g[0]),
                gr - i16::from(g[1]),
                b - i16::from(g[2]),
            ];
            let mine = hue(d, 7);
            let same = key.iter().zip(mine).all(|(&k, m)| k == 0 || k == m);
            if off(p, g) && same {
                mask[row][col] = true;
                found = Some(found.map_or([col, row, col, row], |[l, t, r, b]| {
                    [l.min(col), t.min(row), r.max(col), b.max(row)]
                }));
            }
        }
    }
    let [left, top, right, bottom] = found?;
    let tall = bottom - top + 1;
    if 2 * tall > height {
        return None;
    }
    let right_margin = width - 1 - right;
    let word: Vec<Vec<bool>> = mask[top..=bottom]
        .iter()
        .map(|r| r[left..=right].to_vec())
        .collect();
    Some(Ink {
        bounds: [left, top, right - left + 1, tall],
        brightest: [brightest[0], brightest[1], brightest[2]],
        centred: left.abs_diff(right_margin) <= CENTRED,
        stem: stem(&word),
    })
}

/// The largest size whose cap height rounds to no more than `ink_h` px:
/// `size * CAP / EM < ink_h + 1/2`.
pub(crate) fn size_for(ink_h: u16) -> u16 {
    let limit = (2 * u32::from(ink_h) + 1) * EM_UNITS;
    u16::try_from((limit - 1) / (2 * CAP_UNITS)).unwrap_or(u16::MAX)
}

/// The advance of `word` in Tahoma at `weight`, font units: a letter
/// outside ASCII 32-126 takes a full em.
pub(crate) fn word_units(word: &str, weight: u16) -> u32 {
    let advances = if weight >= BOLD_WEIGHT {
        &BOLD_ADVANCES
    } else {
        &REGULAR
    };
    word.chars()
        .map(|c| {
            u32::from(c)
                .checked_sub(32)
                .and_then(|i| advances.get(usize::try_from(i).ok()?))
                .copied()
                .unwrap_or(EM_UNITS)
        })
        .sum()
}

/// Whether `word` at `weight` and `size` px fits `room` px: its exact
/// advance, no rounding or margin.
pub(crate) fn fits(word: &str, weight: u16, size: u16, room: u16) -> bool {
    u64::from(word_units(word, weight)) * u64::from(size) <= u64::from(room) * u64::from(EM_UNITS)
}

/// The served style of `label` on a node `w x h` px whose copied word's
/// ink, in the node's box, is `ink` at `[x, y, iw, ih]`.
fn style(label: &NodeLabel, width: u16, height: u16, ink: Ink, at: [u16; 4]) -> LabelStyle {
    let [ink_x, ink_y, _, ink_h] = at;
    let (bx, bw, align) = if ink.centred {
        let frame = (width / 10).max(3);
        (frame, width.saturating_sub(2 * frame), LabelAlign::Centre)
    } else {
        (ink_x, width.saturating_sub(ink_x), LabelAlign::Start)
    };
    let weight = weight_for(ink.stem);
    let measured = size_for(ink_h);
    let size = (1..=measured)
        .rev()
        .find(|&px| fits(&label.text, weight, px, bw))
        .unwrap_or(measured);
    let line = (size + 4).min(height);
    let middle = ink_y + ink_h / 2;
    let top = middle.saturating_sub(line / 2).min(height - line);
    let [red, green, blue] = ink.brightest;
    LabelStyle {
        size_px: size,
        weight,
        color: format!("#{red:02X}{green:02X}{blue:02X}"),
        align,
        text_box: [bx, top, bw, line],
    }
}

/// The cover and style of the worded Communities clone `n`, from its art.
fn measure(
    n: &SkinNode,
    package: &Package<'_>,
) -> Result<Option<([u16; 4], LabelStyle)>, SkinError> {
    let (NodeSpan::Fixed { size: width, .. }, NodeSpan::Fixed { size: height, .. }) =
        (n.x.clone(), n.y.clone())
    else {
        return Ok(None);
    };
    let Some(label) = n.label.as_ref() else {
        return Ok(None);
    };
    let Some((frame, dx, dy)) = picture_frame(n, package)? else {
        return Ok(None);
    };
    let Some(ink) = ink(&frame) else {
        return Ok(None);
    };
    let [left, top, across, down] = ink.bounds;
    let side = |value: usize, by: i32| {
        i32::try_from(value)
            .ok()
            .and_then(|value| u16::try_from(value + by).ok())
    };
    let span = |value: usize| u16::try_from(value).ok();
    let (Some(ink_x), Some(ink_y), Some(ink_w), Some(ink_h)) =
        (side(left, dx), side(top, dy), span(across), span(down))
    else {
        return Ok(None);
    };
    let at = [ink_x, ink_y, ink_w, ink_h];
    Ok(Some((at, style(label, width, height, ink, at))))
}

fn walk(nodes: &mut [SkinNode], package: &Package<'_>) -> Result<(), SkinError> {
    for n in nodes {
        let worded = n.role == NodeRole::Tab(ViewKey::Communities)
            && n.label
                .as_ref()
                .is_some_and(|l| l.cover.is_some() && l.glyph.is_none());
        if worded {
            if let Some((cover, style)) = measure(n, package)? {
                if let Some(l) = n.label.as_mut() {
                    l.cover = Some(cover);
                    l.style = Some(style);
                }
            }
        }
        walk(&mut n.children, package)?;
    }
    Ok(())
}

/// Serves the measured style, and the copied word's ink box as its cover,
/// on the worded Communities clone in `layout`'s tree and popups.
///
/// # Errors
/// The clone's picture missing from the package or not decoding.
pub fn worded(layout: &mut WindowLayout, package: &Package<'_>) -> Result<(), SkinError> {
    each_root(layout, |root| walk(root, package))
}
