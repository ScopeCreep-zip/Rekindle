//! TabSRMM `.tsk` (`tabsrmm.tsk`, the engine's documented sample): item
//! records `[%Item]` and image items `[$Name]`.

use rekindle_image::Rgba;
use rekindle_types::dto::skin::RegionState;

use super::super::walk;
use super::ini::{Ini, Section};
use super::{fade, hex_rgb, insets, load, number, Parsed, SkinColour, Slice, Tree};
use crate::import::xfire_xml::bitmap::key_out;
use crate::import::ImportError;

/// The colour keys of an item record (`tabsrmm.tsk:42-45`).
pub const COLOUR_KEYS: [&str; 3] = ["COLOR1", "COLOR2", "TEXTCOLOR"];

/// Item colours with a registry role: (item, key, token). `Frame` is the
/// titlebar and border with a `GRADIENT` from `COLOR1` to `COLOR2`
/// (`:23`, `:104-114`); every other item colour is unmapped.
pub const COLOURS: [(&str, &str, &str); 9] = [
    ("Container", "COLOR1", "bg.panel"),
    ("Container", "TEXTCOLOR", "text.primary"),
    ("MessageLog", "COLOR1", "bg.secondary"),
    ("InputArea", "COLOR1", "bg.input"),
    ("Frame", "COLOR1", "titlebar.start"),
    ("Frame", "COLOR2", "titlebar.end"),
    ("Frame", "TEXTCOLOR", "titlebar.text"),
    ("Statusbarpanel", "COLOR1", "bg.status"),
    ("Statusbarpanel", "TEXTCOLOR", "text.status"),
];

/// Items whose image fills a region slot state, in priority order (the
/// window frame from `Frame`, else from `Container`).
pub const REGIONS: [(&str, &str, RegionState); 6] = [
    ("Frame", "window.frame", RegionState::Normal),
    ("Container", "window.frame", RegionState::Normal),
    ("Statusbarpanel", "statusbar", RegionState::Normal),
    ("Titlebutton", "titlebar.buttons", RegionState::Normal),
    (
        "Titlebuttonmouseover",
        "titlebar.buttons",
        RegionState::Hover,
    ),
    ("Titlebuttonpressed", "titlebar.buttons", RegionState::Down),
];

/// The image-item keys read; `Perpixel` is the PNG's own alpha.
const IMAGE_KEYS: [&str; 9] = [
    "Image",
    "Left",
    "Top",
    "Right",
    "Bottom",
    "Alpha",
    "ColorKey",
    "FillColor",
    "Perpixel",
];

fn item_colours(item: &str, section: &Section, out: &mut Parsed) -> Result<(), ImportError> {
    for e in &section.entries {
        let Some(key) = COLOUR_KEYS
            .into_iter()
            .find(|k| k.eq_ignore_ascii_case(&e.key))
        else {
            out.dropped.push(format!(
                "[%{item}] {}={}: no registry role (alpha, gradient, corners, radius, margins)",
                e.key, e.value
            ));
            continue;
        };
        let value = hex_rgb(&format!("%{item} {key}"), &e.value)?;
        let token = COLOURS
            .iter()
            .find(|(i, k, _)| i.eq_ignore_ascii_case(item) && *k == key)
            .map(|(_, _, t)| *t);
        if let Some(t) = token {
            out.tokens.insert(t.to_owned(), walk::color(value));
        }
        out.colours.push(SkinColour {
            item: format!("%{item}"),
            key: key.to_owned(),
            raw: e.value.clone(),
            token,
        });
    }
    Ok(())
}

/// The items `section` assigns (`Item0=…`, `Item1=…`, `tabsrmm.tsk:333-338`).
fn assigned(section: &Section) -> impl Iterator<Item = &str> {
    section
        .entries
        .iter()
        .filter(|e| is_item_key(&e.key))
        .map(|e| e.value.as_str())
}

/// `Item<N>`.
fn is_item_key(key: &str) -> bool {
    key.get(..4).is_some_and(|n| n.eq_ignore_ascii_case("Item"))
        && key
            .get(4..)
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))
}

fn margins(file: &str, s: &Section) -> Result<Option<[u32; 4]>, ImportError> {
    let mut m = [0; 4];
    for (slot, key) in m.iter_mut().zip(["Left", "Top", "Right", "Bottom"]) {
        let Some(v) = s.get(key) else {
            return Ok(None);
        };
        *slot = number(file, &format!("[{}] {key}", s.name), v)?;
    }
    Ok(Some(m))
}

/// The slice the image item `s` gives `item`, or `None` (named in
/// `out.dropped`).
fn slice(
    tree: &Tree,
    file: &str,
    s: &Section,
    (item, slot, state): (&str, &'static str, RegionState),
    out: &mut Parsed,
) -> Result<Option<Slice>, ImportError> {
    let name = &s.name;
    let Some(raw) = s.get("Image") else {
        out.dropped.push(format!("[{name}]: no Image="));
        return Ok(None);
    };
    let Some(m) = margins(file, s)? else {
        out.dropped.push(format!(
            "[{name}] Image={raw}: not divided (sizing margins are used only when all four are given)"
        ));
        return Ok(None);
    };
    let (key, mut image) = match load(tree, raw, true) {
        Ok(l) => l,
        Err(why) => {
            out.dropped.push(format!("[{name}] Image={raw}: {why}"));
            return Ok(None);
        }
    };
    out.files.insert(key.clone());
    let source = format!("[{name}] {key} as {item}");
    let insets = insets(&source, image.dimensions(), m)?;
    if let Some(a) = s.get("Alpha") {
        let a = number(file, &format!("[{name}] Alpha"), a)?;
        if a > 100 {
            return Err(ImportError::Format {
                file: file.to_owned(),
                detail: format!("[{name}] Alpha={a} is not a percent"),
            });
        }
        fade(&mut image, a, 100);
    }
    // The colour key only makes sense for the container (`:340-346`).
    if let Some(c) = s.get("ColorKey") {
        if item.eq_ignore_ascii_case("Container") {
            key_out(&mut image, hex_rgb(&format!("{name} ColorKey"), c)?);
        } else {
            out.dropped
                .push(format!("[{name}] ColorKey: {item} is not the container"));
        }
    }
    if let Some(c) = s.get("FillColor") {
        let [r, g, b] = hex_rgb(&format!("{name} FillColor"), c)?;
        let (w, h) = image.dimensions();
        let (l, t, rt, bt) = (m[0], m[1], m[2], m[3]);
        for y in t..h - bt {
            for x in l..w - rt {
                image.put_pixel(x, y, Rgba([r, g, b, 255]));
            }
        }
    }
    Ok(Some(Slice {
        slot,
        state,
        source,
        image,
        insets,
    }))
}

/// Reads the `.tsk` `ini` (the file `file` of `tree`).
pub(super) fn parse(tree: &Tree, file: &str, ini: &Ini) -> Result<Parsed, ImportError> {
    let mut out = Parsed::default();
    let mut images: Vec<&Section> = Vec::new();
    for s in &ini.sections {
        if let Some(item) = s.name.strip_prefix('%') {
            item_colours(item, s, &mut out)?;
        } else if s.name.starts_with('$') {
            images.push(s);
        } else {
            out.dropped.push(format!(
                "[{}]: neither a TabSRMM item nor an image item",
                s.name
            ));
        }
    }
    let owner = |item: &str| {
        images
            .iter()
            .find(|s| assigned(s).any(|a| a.eq_ignore_ascii_case(item)))
    };
    for (item, slot, state) in REGIONS {
        if let Some(s) = owner(item) {
            if let Some(sl) = slice(tree, file, s, (item, slot, state), &mut out)? {
                out.slices.push(sl);
            }
        }
    }
    for s in &images {
        for a in assigned(s) {
            let why = if !REGIONS.iter().any(|(i, _, _)| i.eq_ignore_ascii_case(a)) {
                "no region slot".to_owned()
            } else if let Some(first) = owner(a).filter(|o| o.name != s.name) {
                format!("already assigned by [{}]", first.name)
            } else {
                continue;
            };
            out.dropped.push(format!("[{}] Item={a}: {why}", s.name));
        }
        for e in &s.entries {
            let known =
                IMAGE_KEYS.iter().any(|k| k.eq_ignore_ascii_case(&e.key)) || is_item_key(&e.key);
            if !known {
                out.dropped
                    .push(format!("[{}] {}: not a TabSRMM image key", s.name, e.key));
            }
        }
    }
    if let Some(stem) = file.rsplit('/').next() {
        out.info.insert("Name".to_owned(), stem.to_owned());
    }
    Ok(out)
}
