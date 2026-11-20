//! Clist_modern `.msf` (`clist_modern_skin.msf`, a documented sample):
//! `[Skin_Description_Section]` metadata and `[ModernSkin]` objects.
//!
//! A glyph is `sGlyph,Fragment,<file>,<x>,<y>,<w>,<h>,<mode>,<left>,<top>,
//! <right>,<bottom>,<alpha>`: an atlas fragment, its stretch mode, four
//! margins and a 0-255 alpha. The margins are read left, top, right,
//! bottom: the sample's `$$Window` fragment is 100x50 with margins
//! 11,24,12,12 (`:22`), a 24 px titlebar, and its `[CLUI]` client margins
//! are Left 11, Top 25, Right 12, Bottom 12 (`:148-152`).

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::RegionState;

use super::ini::{Ini, KeyValue};
use super::{fade, insets, load, number, Parsed, SkinColour, Slice, Tree};
use crate::import::ImportError;

/// Glyph objects that fill a region slot state (plan E7.7.7 S4 part 3:
/// the sample's group row `$$Group/Open` with its `/Hot` image (`:24-25`,
/// selected by `@23`/`@25`, `:90-92`), the selected contact row (`:41`,
/// `@50`), the bar buttons (`:75-77`, `@1610-1630`) and the scrollbar
/// parts (`:46-49`, `@80-110`)).
pub const GLYPHS: [(&str, &str, RegionState); 16] = [
    ("$$Window", "window.frame", RegionState::Normal),
    ("$$FrameTitleBar", "titlebar", RegionState::Normal),
    ("$$StatusBar", "statusbar", RegionState::Normal),
    (
        "$$Button/Minimize/Normal",
        "titlebar.buttons",
        RegionState::Normal,
    ),
    (
        "$$Button/Minimize/Hovered",
        "titlebar.buttons",
        RegionState::Hover,
    ),
    (
        "$$Button/Minimize/Down",
        "titlebar.buttons",
        RegionState::Down,
    ),
    (
        "$$Group/Open",
        "buddylist.group-header",
        RegionState::Normal,
    ),
    (
        "$$Group/Open/Hot",
        "buddylist.group-header",
        RegionState::Hover,
    ),
    (
        "$$Contact/Selected",
        "buddylist.row-selected",
        RegionState::Normal,
    ),
    (
        "$$BarButton",
        "buddylist.action-button",
        RegionState::Normal,
    ),
    (
        "$$BarButton/Hot",
        "buddylist.action-button",
        RegionState::Hover,
    ),
    (
        "$$BarButton/Pressed",
        "buddylist.action-button",
        RegionState::Down,
    ),
    ("$$Scroll/Back", "scrollbar.track", RegionState::Normal),
    ("$$Scroll/Thumb", "scrollbar.thumb", RegionState::Normal),
    ("$$Scroll/BtnUp", "scrollbar.button-up", RegionState::Normal),
    (
        "$$Scroll/BtnDn",
        "scrollbar.button-down",
        RegionState::Normal,
    ),
];

/// The one stretch mode a nine-slice reproduces.
pub const STRETCH: &str = "StretchBoth";
/// The fields of a fragment glyph.
const FRAGMENT_FIELDS: usize = 13;

fn bad(file: &str, e: &KeyValue, detail: &str) -> ImportError {
    ImportError::Format {
        file: file.to_owned(),
        detail: format!("line {}: {}: {detail}", e.line, e.key),
    }
}

/// The `w`×`h` fragment at (`x`, `y`) of `atlas`, refused when it leaves
/// the image.
fn crop(source: &str, atlas: &RgbaImage, [x, y, w, h]: [u32; 4]) -> Result<RgbaImage, ImportError> {
    let (aw, ah) = atlas.dimensions();
    if w == 0 || h == 0 || x.saturating_add(w) > aw || y.saturating_add(h) > ah {
        return Err(ImportError::Image {
            file: source.to_owned(),
            detail: format!("fragment {x},{y} {w}x{h} lies outside the {aw}x{ah} image"),
        });
    }
    Ok(RgbaImage::from_fn(w, h, |i, j| {
        *atlas.get_pixel(x + i, y + j)
    }))
}

/// The slice the glyph `e` gives `(slot, state)`, or `None` (named).
fn glyph(
    tree: &Tree,
    file: &str,
    e: &KeyValue,
    (slot, state): (&'static str, RegionState),
    out: &mut Parsed,
) -> Result<Option<Slice>, ImportError> {
    let f: Vec<&str> = e.value.split(',').map(str::trim).collect();
    if !f[0].eq_ignore_ascii_case("sGlyph") {
        return Err(bad(file, e, "not an sGlyph object"));
    }
    if !f.get(1).is_some_and(|k| k.eq_ignore_ascii_case("Fragment")) {
        let kind = f.get(1).copied().unwrap_or_default();
        out.dropped
            .push(format!("{}: sGlyph,{kind} carries no image", e.key));
        return Ok(None);
    }
    if f.len() != FRAGMENT_FIELDS {
        return Err(bad(file, e, "a fragment glyph has 13 fields"));
    }
    let n = |i: usize| number(file, &e.key, f[i]);
    let rect = [n(3)?, n(4)?, n(5)?, n(6)?];
    let margins = [n(8)?, n(9)?, n(10)?, n(11)?];
    let alpha = n(12)?;
    if alpha > 255 {
        return Err(bad(file, e, "alpha is 0-255"));
    }
    if !f[7].eq_ignore_ascii_case(STRETCH) {
        out.dropped.push(format!(
            "{}: mode {} (a nine-slice reproduces only {STRETCH})",
            e.key, f[7]
        ));
        return Ok(None);
    }
    let (key, atlas) = match load(tree, f[2], false) {
        Ok(l) => l,
        Err(why) => {
            out.dropped.push(format!("{}: {why}", e.key));
            return Ok(None);
        }
    };
    out.files.insert(key.clone());
    let source = format!("{} {key}", e.key);
    let mut image = crop(&source, &atlas, rect)?;
    let insets = insets(&source, image.dimensions(), margins)?;
    fade(&mut image, alpha, 255);
    Ok(Some(Slice {
        slot,
        state,
        source,
        image,
        insets,
    }))
}

/// A `t` text object's colour (`:18`, field 8, `0xAARRGGBB`), unmapped: it
/// colours text drawn over a glyph, which has no registry role.
fn text_colour(e: &KeyValue, out: &mut Parsed) {
    let raw = e.value.split(',').nth(7).map(str::trim).unwrap_or_default();
    out.colours.push(SkinColour {
        item: e.key.clone(),
        key: "text object colour".to_owned(),
        raw: raw.to_owned(),
        token: None,
    });
}

fn modern_skin(
    tree: &Tree,
    file: &str,
    entries: &[KeyValue],
    out: &mut Parsed,
) -> Result<(), ImportError> {
    for e in entries {
        let k = e.key.as_str();
        if k.starts_with("$$") {
            match GLYPHS.iter().find(|(g, _, _)| g.eq_ignore_ascii_case(k)) {
                Some((_, slot, state)) => {
                    if let Some(s) = glyph(tree, file, e, (slot, *state), out)? {
                        out.slices.push(s);
                    }
                }
                None => out.dropped.push(format!("{k}: no region slot")),
            }
        } else if k.starts_with('t') && e.value.starts_with("s$$") {
            text_colour(e, out);
        } else {
            let kind = match k.chars().next() {
                Some('f') => "a font",
                Some('@') => "an object selector",
                Some('#') => "a button",
                _ => "an unknown object",
            };
            out.dropped.push(format!("{k}: {kind}"));
        }
    }
    Ok(())
}

/// Reads the `.msf` `ini` (the file `file` of `tree`).
pub(super) fn parse(tree: &Tree, file: &str, ini: &Ini) -> Result<Parsed, ImportError> {
    let mut out = Parsed::default();
    for s in &ini.sections {
        if s.name.eq_ignore_ascii_case("Skin_Description_Section") {
            for e in s.entries.iter().filter(|e| !e.value.is_empty()) {
                out.info
                    .insert(e.key.clone(), e.value.chars().take(120).collect());
            }
        } else if s.name.eq_ignore_ascii_case("ModernSkin") {
            modern_skin(tree, file, &s.entries, &mut out)?;
        } else {
            for e in &s.entries {
                out.dropped
                    .push(format!("[{}] {}: a contact-list setting", s.name, e.key));
            }
        }
    }
    Ok(out)
}
