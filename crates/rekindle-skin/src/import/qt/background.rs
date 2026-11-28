//! The QSS `background-image` reader (plan E7.7.7 S4 part 3): a Mumble or
//! TeamSpeak 3 stylesheet's window, list, button, status-bar and scrollbar
//! backgrounds onto the region slots of the same parts.
//!
//! A skin stays declarative (plan E7.7.7 *Design*; the skins study's
//! security rules: no CSS, HTML, scripts or URLs in a package): the QSS is
//! never applied. Only `background-image: url(…)` on a whole widget type is
//! read, and only when the URL names a file inside the theme through the
//! theme's own search path (`skin:` for Mumble, `styles:` for TeamSpeak 3,
//! `a-legacy-formats.md` §5); every other URL (a scheme, `data:`, a Qt
//! resource `:/`, an absolute or `..` path, a path relative to the
//! program's working directory) is refused and named. The image goes
//! through [`images::check`] (its header's size is checked before any
//! pixel is allocated) or the reference-refusing [`svg`] rasteriser.
//! Bounds are checked before allocation: the stylesheet is within the
//! legacy limits (`check_limits`), at most [`MAX_RULES`] rules are read,
//! a URL is at most [`MAX_URL`] bytes and at most [`MAX_BACKGROUNDS`]
//! backgrounds are taken.
//!
//! Qt repeats a `background-image` (`background-repeat` defaults to
//! `repeat`), so each region is [`RegionFit::Tile`].

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{RegionFit, RegionSpec, RegionState, SkinError};

use super::{svg, Front, Read};
use crate::images;
use crate::import::pidgin::map::plain_path;
use crate::import::Tree;

/// The most `selector { … }` rules read from one stylesheet.
pub const MAX_RULES: usize = 4096;
/// The longest `url(…)` argument read, in bytes.
pub const MAX_URL: usize = 256;
/// The most backgrounds taken from one stylesheet.
pub const MAX_BACKGROUNDS: usize = 64;

/// Widget selectors (without a state) → slot, in priority order: a slot's
/// state takes the first row that gives it. A list view has no slot of its
/// own; Rekindle's buddy list shows the window behind it.
pub const SELECTORS: [(&str, &str); 14] = [
    ("QMainWindow", "window.frame"),
    ("QDialog", "window.frame"),
    ("QTreeView", "window.frame"),
    ("QListView", "window.frame"),
    ("QTreeWidget", "window.frame"),
    ("QListWidget", "window.frame"),
    ("QPushButton", "button.secondary"),
    ("QStatusBar", "statusbar"),
    ("QScrollBar::handle:vertical", "scrollbar.thumb"),
    (
        "QScrollBar::handle:horizontal",
        "scrollbar.thumb-horizontal",
    ),
    ("QScrollBar::sub-line:vertical", "scrollbar.button-up"),
    ("QScrollBar::add-line:vertical", "scrollbar.button-down"),
    ("QScrollBar:vertical", "scrollbar.track"),
    ("QScrollBar:horizontal", "scrollbar.track-horizontal"),
];

/// Qt pseudo-states → region states.
const STATES: [(&str, RegionState); 5] = [
    ("", RegionState::Normal),
    (":hover", RegionState::Hover),
    (":pressed", RegionState::Down),
    (":disabled", RegionState::Disabled),
    (":focus", RegionState::Focused),
];

/// One region slot's images: state → (package path, canonical PNG, source).
pub type SlotImages = BTreeMap<RegionState, (String, Vec<u8>, String)>;

/// `selector` as (slot, state, priority), or `None`.
fn slot_of(selector: &str) -> Option<(&'static str, RegionState, usize)> {
    SELECTORS.iter().enumerate().find_map(|(i, (sel, slot))| {
        let rest = selector.strip_prefix(sel)?;
        let (_, state) = STATES.iter().find(|(p, _)| *p == rest)?;
        Some((*slot, *state, i))
    })
}

/// `text` without its `/* … */` comments.
fn uncomment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        match rest[at + 2..].find("*/") {
            Some(end) => rest = &rest[at + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The `url(…)` argument of a `background-image` value, unquoted.
fn url_arg(value: &str) -> Option<&str> {
    let inner = value.trim().strip_prefix("url(")?.strip_suffix(')')?.trim();
    Some(inner.trim_matches(|c| c == '"' || c == '\''))
}

/// The `;`-separated declarations of a rule body, a `;` inside `( … )`
/// (a `data:` URL's media type) kept in its declaration.
fn declarations(body: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0usize;
    let mut start = 0;
    let mut out = Vec::new();
    for (i, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out.into_iter()
}

/// The tree path `url` names, or why it is refused.
pub fn resolve(front: Front, root: &str, url: &str) -> Result<String, String> {
    if url.len() > MAX_URL {
        return Err(format!("a URL longer than {MAX_URL} bytes"));
    }
    let (prefix, base) = match front {
        Front::Mumble => ("skin:", root.to_owned()),
        Front::TeamSpeak3 => ("styles:", format!("{root}styles/")),
    };
    let Some(rel) = url.strip_prefix(prefix) else {
        return Err(format!(
            "not a file inside the theme (only {prefix}<path> is read; a scheme, data:, a Qt \
             resource or a path relative to the program is refused)"
        ));
    };
    if !plain_path(rel) {
        return Err("a path that leaves the theme".to_owned());
    }
    Ok(format!("{base}{rel}"))
}

/// The canonical PNG of the image at `path`, or why it is dropped.
fn image(tree: &Tree, path: &str, out: &str) -> Result<Vec<u8>, String> {
    let bytes = tree.get(path).ok_or("no such file in the theme")?;
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let png = match ext.as_deref() {
        Some("png") => bytes.clone(),
        Some("svg") => {
            svg::rasterise(bytes)
                .map_err(|why| format!("SVG refused: {why}"))?
                .png
        }
        _ => return Err("not PNG or SVG".to_owned()),
    };
    images::check(out, &png)
        .map_err(|rule| format!("refused by the validator: {}", SkinError::new(out, rule)))
}

/// The package path of `slot`'s `state` image.
fn package_path(slot: &str, state: RegionState) -> String {
    let slug = slot.replace('.', "-");
    match state {
        RegionState::Normal => format!("assets/{slug}.png"),
        s => format!("assets/{slug}--{}.png", s.key()),
    }
}

/// Every whole-widget `background-image` of `qss` (the file `file`), into
/// `read.regions`; every other one is named in `read.dropped`.
pub fn read(tree: &Tree, read: &mut Read, root: &str, file: &str, qss: &str) {
    let text = uncomment(qss);
    let mut found: Vec<(usize, &'static str, RegionState, String)> = Vec::new();
    for (n, rule) in text.split('}').enumerate() {
        if n == MAX_RULES {
            read.drop(file, format!("dropped: rules past the first {MAX_RULES}"));
            break;
        }
        let Some((selectors, body)) = rule.split_once('{') else {
            continue;
        };
        let values: Vec<&str> = declarations(body)
            .filter_map(|d| d.split_once(':'))
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("background-image"))
            .map(|(_, v)| v)
            .collect();
        let Some(value) = values.first() else {
            continue;
        };
        let Some(url) = url_arg(value) else {
            read.drop(
                format!("{file} {} background-image", selectors.trim()),
                "dropped: not one url(...)",
            );
            continue;
        };
        for sel in selectors.split(',').map(str::trim) {
            let item = format!("{file} {sel} background-image");
            if found.len() == MAX_BACKGROUNDS {
                read.drop(item, format!("dropped: past {MAX_BACKGROUNDS} backgrounds"));
                continue;
            }
            let Some((slot, state, prio)) = slot_of(sel) else {
                read.drop(item, "dropped: not a whole widget with a region slot");
                continue;
            };
            match resolve(read.front, root, url) {
                Ok(path) => found.push((prio, slot, state, path)),
                Err(why) => read.drop(item, format!("dropped: url refused: {why}")),
            }
        }
    }
    found.sort_by_key(|(prio, ..)| *prio);
    for (_, slot, state, path) in found {
        let item = format!("{path} as {slot} {}", state.key());
        let slot_images = read.regions.entry(slot).or_default();
        if slot_images.contains_key(&state) {
            read.drop(item, "dropped: the state is already set");
            continue;
        }
        let out = package_path(slot, state);
        match image(tree, &path, &out) {
            Ok(png) => {
                slot_images.insert(state, (out, png, path));
            }
            Err(why) => read.drop(item, format!("dropped: {why}")),
        }
    }
    // A state image joins only a slot with a normal image.
    let orphans: Vec<&'static str> = read
        .regions
        .iter()
        .filter(|(_, images)| !images.contains_key(&RegionState::Normal))
        .map(|(slot, _)| *slot)
        .collect();
    for slot in orphans {
        for (state, (_, _, path)) in read.regions.remove(slot).unwrap_or_default() {
            read.drop(
                format!("{path} as {slot} {}", state.key()),
                "dropped: no normal image for the slot",
            );
        }
    }
}

/// The manifest region of `slot`.
#[must_use]
pub fn spec(slot: &str, images: &SlotImages) -> RegionSpec {
    RegionSpec {
        image: package_path(slot, RegionState::Normal),
        fit: RegionFit::Tile,
        states: images
            .keys()
            .filter(|s| **s != RegionState::Normal)
            .map(|s| (*s, package_path(slot, *s)))
            .collect(),
        tint: None,
        mask: None,
        // A text slot's backing is the first ground its text is declared on
        // (plan E7.7.7), as `import::Control::spec`.
        text_backing: crate::regions::slot_of(slot)
            .filter(|s| !s.text.is_empty())
            .and_then(|s| s.grounds.first())
            .map(|g| (*g).to_owned()),
    }
}
