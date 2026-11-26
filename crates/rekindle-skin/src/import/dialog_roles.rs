//! The dialog roles an Xfire import takes from the pack's art (plan E7.7.3
//! item 1, E7.7.7; research `research-settings-theme-2026-11-26.md` §3.2,
//! §3.3; ST1 rulings 3, 4, 2026-11-26). A role the pack's theme names
//! comes from the colour tables; a role it neither names nor draws is
//! derived after the base walk by the registry's one table
//! (`registry::dialog::fill`, inside `walk::settle`).
//!
//! Two grounds are the alpha-weighted mean colour of the pack's own art,
//! never a registry default: the title bar (`titlebar.start`/`end`), the
//! two ends of the buddy-list title strip (the `titlebar` control the
//! surface table fills from `MainTopRow` or `MainTitleBar`,
//! `xfire_xml/surfaces.rs`; Classic v3 and the INI era's left/middle/right
//! bars), decoded by `controls::read` into one image whose nine-slice left
//! and right insets are its end pieces; and the menu bar (`menubar.bg`),
//! the whole `buddylist.menubar` control (`MenuBackground`, `MenuBar`), the
//! ground Xfire draws `MenuTextCG` on, or, for a pack with no menu-bar
//! control, the art its menu text is laid on (an INI main window lays its
//! menu titles on its frame, `xfire_ini/menu_ground.rs`). Only the colours leave the importer,
//! never the art (Settings draws none, plan SB1).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionFit, RegionState, TokenValue};

use super::Control;

/// The alpha-weighted mean colour of `cols` × `rows` of `img` (a
/// transparent pixel weighs nothing); `None` when every pixel there is
/// transparent.
#[must_use]
pub fn region_mean(
    img: &RgbaImage,
    cols: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) -> Option<[u8; 3]> {
    let (mut sum, mut weight) = ([0u64; 3], 0u64);
    for x in cols {
        for y in rows.clone() {
            let p = img.get_pixel(x, y).0;
            let a = u64::from(p[3]);
            for (s, c) in sum.iter_mut().zip(p) {
                *s += u64::from(c) * a;
            }
            weight += a;
        }
    }
    (weight > 0).then(|| sum.map(|s| u8::try_from((s + weight / 2) / weight).unwrap_or(u8::MAX)))
}

/// The alpha-weighted mean colour of columns `cols` of `img`, every row.
fn mean(img: &RgbaImage, cols: std::ops::Range<u32>) -> Option<[u8; 3]> {
    region_mean(img, cols, 0..img.height())
}

/// The mean colours of the strip's left and right ends: its nine-slice end
/// insets, or its outer eighths when it has none.
#[must_use]
pub fn strip_ends(img: &RgbaImage, fit: RegionFit) -> Option<([u8; 3], [u8; 3])> {
    let w = img.width();
    let eighth = (w / 8).max(1);
    let (left, right) = match fit {
        RegionFit::NineSlice(i) if i.left > 0 && i.right > 0 => {
            (u32::from(i.left).min(w), u32::from(i.right).min(w))
        }
        _ => (eighth.min(w), eighth.min(w)),
    };
    Some((mean(img, 0..left)?, mean(img, w - right..w)?))
}

/// The alpha-weighted mean colour of the whole of `img`.
#[must_use]
pub fn art_mean(img: &RgbaImage) -> Option<[u8; 3]> {
    mean(img, 0..img.width())
}

/// The Normal image of the control filling `region`.
fn normal<'c>(controls: &'c [Control], region: &str) -> Option<(&'c RgbaImage, RegionFit)> {
    let c = controls.iter().find(|c| c.region == region)?;
    Some((c.images.get(&RegionState::Normal)?, c.fit))
}

/// `tokens` with the grounds the pack draws as art and does not name:
/// the title strip's two ends and the menu bar's mean, or, with no menu-bar
/// control, `menu_text_ground` (the mean of the art the pack lays its menu
/// text on).
#[must_use]
pub fn with_art(
    mut tokens: BTreeMap<String, TokenValue>,
    controls: &[Control],
    menu_text_ground: Option<[u8; 3]>,
) -> BTreeMap<String, TokenValue> {
    let strip = normal(controls, "titlebar").and_then(|(img, fit)| strip_ends(img, fit));
    let menu = normal(controls, "buddylist.menubar")
        .and_then(|(img, _)| art_mean(img))
        .or(menu_text_ground);
    let grounds = strip
        .map(|(start, end)| [("titlebar.start", start), ("titlebar.end", end)])
        .into_iter()
        .flatten()
        .chain(menu.map(|c| ("menubar.bg", c)));
    for (id, c) in grounds {
        tokens
            .entry(id.to_owned())
            .or_insert_with(|| super::walk::color(c));
    }
    tokens
}
