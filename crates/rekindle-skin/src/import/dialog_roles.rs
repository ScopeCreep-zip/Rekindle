//! The dialog roles an Xfire import writes for every pack (plan E7.7.3
//! item 1, E7.7.7; research `research-settings-theme-2026-11-26.md` §3.2,
//! §3.3): a pack's own colour when its theme names one (the colour tables
//! map it), else the derivation in [`DERIVE`] over the colours it does
//! define. A registry default never stands in for a pack colour.
//!
//! The title-bar ground (`titlebar.start`/`end`) is the alpha-weighted mean
//! colour of the two ends of the pack's own buddy-list title strip: the
//! `titlebar` control the surface table fills from `MainTopRow` or
//! `MainTitleBar` (`xfire_xml/surfaces.rs`; Classic v3 and the INI era's
//! left/middle/right bars), decoded by `controls::read` into one image whose
//! nine-slice left and right insets are its end pieces. Only the two
//! colours leave the importer, never the art (Settings draws none, plan
//! SB1). A pack with no strip takes its selection colour toward its ground.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionFit, RegionState, TokenValue};

use super::Control;
use crate::registry::Derivation;

const fn mix(a: &'static str, b: &'static str, a_weight: u16) -> Derivation {
    Derivation::Mix { a, b, a_weight }
}

const fn same(of: &'static str) -> Derivation {
    mix(of, of, 1000)
}

/// Each role and its derivation from tokens an Xfire import sets
/// (`bg.primary` ← `ListBGCG`/`BuddyViewBG`, `bg.panel` ← the chat view,
/// `bg.secondary` ← the info view, `text.base` ← the friend text,
/// `accent.fill` ← the selected row), in order: a later row may read an
/// earlier role.
pub const DERIVE: &[(&str, Derivation)] = &[
    // The INI era's edit box takes its chat view's colour group (Combat's
    // `ColorGroup=ChatBGCG`), as `bg.input` follows `bg.panel`.
    ("input.bg", same("bg.panel")),
    ("input.text", same("text.base")),
    // Xfire selects with its selected-row colour.
    ("input.selection.bg", same("accent.fill")),
    (
        "input.selection.bg-inactive",
        mix("accent.fill", "input.bg", 400),
    ),
    ("selection.text", same("text.base")),
    ("menubar.text", same("text.base")),
    // The registry's own `text.disabled` mix, over the pack's list text.
    ("text.disabled", mix("text.base", "bg.primary", 860)),
    ("progress.bg", same("bg.panel")),
    ("progress.fill", same("accent.fill")),
    ("progress.outline", mix("text.base", "progress.bg", 500)),
    ("progress.text", same("text.base")),
    ("scrollbar.track", same("bg.primary")),
    ("scrollbar.thumb", mix("text.base", "scrollbar.track", 500)),
    ("tooltip.bg", same("bg.secondary")),
    ("tooltip.text", same("text.base")),
    ("titlebar.start", mix("accent.fill", "bg.primary", 500)),
    ("titlebar.end", mix("accent.fill", "bg.primary", 250)),
];

/// The alpha-weighted mean colour of columns `cols` of `img`; `None` when
/// every pixel there is transparent.
fn mean(img: &RgbaImage, cols: std::ops::Range<u32>) -> Option<[u8; 3]> {
    let (mut sum, mut weight) = ([0u64; 3], 0u64);
    for x in cols {
        for y in 0..img.height() {
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

/// `tokens` with every dialog role the pack's theme did not name: the
/// title strip's ends from `controls`, then [`DERIVE`] in order. A role
/// whose sources the pack lacks stays unset (the built-in tests refuse it).
#[must_use]
pub fn filled(
    mut tokens: BTreeMap<String, TokenValue>,
    controls: &[Control],
) -> BTreeMap<String, TokenValue> {
    let strip = controls
        .iter()
        .find(|c| c.region == "titlebar")
        .and_then(|c| strip_ends(c.images.get(&RegionState::Normal)?, c.fit));
    if let Some((start, end)) = strip {
        for (id, c) in [("titlebar.start", start), ("titlebar.end", end)] {
            tokens
                .entry(id.to_owned())
                .or_insert_with(|| super::walk::color(c));
        }
    }
    for (id, d) in DERIVE {
        if tokens.contains_key(*id) {
            continue;
        }
        let view: BTreeMap<&str, TokenValue> = tokens
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        let all_set = d.sources().iter().all(|s| view.contains_key(s));
        if let Some(v) = all_set.then(|| d.apply(&view)).flatten() {
            tokens.insert((*id).to_owned(), v);
        }
    }
    tokens
}
