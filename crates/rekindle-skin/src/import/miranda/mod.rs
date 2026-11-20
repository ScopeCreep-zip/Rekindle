//! The Miranda NG importer (plan E7.7.3 item 6): one TabSRMM `.tsk` or one
//! Clist_modern `.msf` skin file and the images it names. Format, from the
//! engines' own documentation (`skins/research/a-legacy-formats.md` §4.1,
//! `.claude/plans/skins/raw/a/miranda/`; behaviour only, no GPL code read
//! or copied):
//!
//! * both are INI files, read by the one bounded [`ini`] reader (a repeated
//!   key or section is read as Windows reads it, the first wins, and a line
//!   without `=` gives no value; each is named);
//! * TabSRMM ([`tabsrmm`]): `[%Item]` records give `COLOR1`, `COLOR2` and
//!   `TEXTCOLOR` (`tabsrmm.tsk:42-64`); `[$Name]` image items give a 32-bit
//!   PNG, four sizing margins that "divide the image into 9 parts" and are
//!   used only when all four are given, a percent `Alpha` applied after
//!   per-pixel alpha, `ItemN=` assignments, a `ColorKey` for the container
//!   only and a centre `FillColor` (`:309-353`);
//! * Clist_modern ([`clist`]): `$$Object = sGlyph,Fragment,<file>,x,y,w,h,
//!   <mode>,<l>,<t>,<r>,<b>,<alpha>` atlas fragments (`clist_modern_skin.msf:22`)
//!   and `t` text objects with an ARGB colour (`:18`).
//!
//! Colours with a registry role become tokens; every other colour is named
//! in the `NOTICE` as unmapped. 9-slice images become the region slots
//! that exist (`window.frame` with its window-inactive image, `titlebar`,
//! `titlebar.buttons`, `statusbar`, the buddy list's group header,
//! selected row and action buttons, the scrollbar parts and
//! `button.secondary`); every other image item is named. Each image is decoded
//! under the validator's image limits (PNG or GIF, and a Clist_modern
//! `*.tga` atlas through the bounded [`tga`] decoder; JPEG is refused as
//! step 4 refuses it; a refused TGA, an SVG or any other type is dropped
//! and named, as is a path that leaves the skin or a missing file); every
//! decoded image is re-encoded as canonical PNG and validated with the
//! package; insets that leave no centre refuse the import. The
//! package goes through [`super::conclude`] (a draft names every failing
//! pair).

mod clist;
mod notice;
mod tabsrmm;
pub mod tga;

use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::{encode_png, kind_of, DynamicImage, ImageKind, RgbaImage};
use rekindle_types::dto::skin::{
    Insets, RegionFit, RegionSpec, RegionState, SkinBase, SkinContrast, SkinLicense, SkinManifest,
    SkinSource, SkinVariant, TokenValue, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

use super::xfire_ini::{ini, lookup};
use super::xfire_xml::bitmap;
use super::{base_of, conclude, credits, tree_hash, ImportError, Outcome, Tree};
use crate::output::{token_file, zip};
use crate::package::MANIFEST;

/// The `source.format` of an imported package.
pub const MIRANDA_FORMAT: &str = "miranda";
const TOKENS: &str = "tokens.json";

/// The token painted under the text of each slot that carries text
/// (`crate::regions::SLOTS`), as the Winamp importer's titlebar; the
/// E7.7.7 slots take the first ground their text is declared on.
pub const TEXT_BACKINGS: [(&str, &str); 6] = [
    ("titlebar", "titlebar.start"),
    ("statusbar", "bg.status"),
    ("buddylist.group-header", "bg.dark"),
    ("buddylist.row-selected", "bg.dark"),
    ("buddylist.action-button", "bg.dark"),
    ("button.secondary", "bg.panel"),
];

/// Which Miranda NG engine the skin file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// TabSRMM message window, `.tsk`.
    TabSrmm,
    /// Clist_modern contact list, `.msf`.
    ClistModern,
}

/// One colour the skin defines and the registry token it became (`None`:
/// unmapped, named in the `NOTICE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinColour {
    pub item: String,
    pub key: String,
    pub raw: String,
    pub token: Option<&'static str>,
}

/// One 9-slice image the skin gives a slot's state.
#[derive(Debug)]
pub struct Slice {
    pub slot: &'static str,
    pub state: RegionState,
    pub source: String,
    pub image: RgbaImage,
    pub insets: Insets,
}

/// One region slot's images: its insets and one image per state.
#[derive(Debug)]
pub struct SlotImages {
    pub insets: Insets,
    pub images: BTreeMap<RegionState, RgbaImage>,
    pub sources: BTreeMap<RegionState, String>,
}

/// What an engine reader found in the skin file.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub info: BTreeMap<String, String>,
    pub colours: Vec<SkinColour>,
    pub tokens: BTreeMap<String, TokenValue>,
    pub slices: Vec<Slice>,
    pub dropped: Vec<String>,
    pub files: BTreeSet<String>,
}

/// What the importer read, before the package is built.
#[derive(Debug)]
pub struct Read {
    pub engine: Engine,
    pub skin_file: String,
    pub info: BTreeMap<String, String>,
    pub colours: Vec<SkinColour>,
    pub tokens: BTreeMap<String, TokenValue>,
    pub regions: BTreeMap<&'static str, SlotImages>,
    /// Every item, key, image and file dropped, each named with why.
    pub dropped: Vec<String>,
    /// Files of the tree that were not read.
    pub unread: Vec<String>,
    /// Repeated keys not read (the first occurrence wins).
    pub repeated: Vec<String>,
}

/// `RRGGBB` (TabSRMM's colour values, `tabsrmm.tsk:72-73`).
pub(crate) fn hex_rgb(item: &str, value: &str) -> Result<[u8; 3], ImportError> {
    let bad = || ImportError::BadColour {
        name: item.to_owned(),
        value: value.to_owned(),
    };
    if value.len() != 6 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let n = u32::from_str_radix(value, 16).map_err(|_| bad())?;
    let [_, r, g, b] = n.to_be_bytes();
    Ok([r, g, b])
}

/// A non-negative pixel count or alpha.
pub(crate) fn number(file: &str, what: &str, value: &str) -> Result<u32, ImportError> {
    value.trim().parse().map_err(|_| ImportError::Format {
        file: file.to_owned(),
        detail: format!("{what}={value} is not a non-negative number"),
    })
}

/// Whether `key` names a TGA (the format has no signature; Clist_modern
/// names its atlases `*.tga`).
fn is_tga(key: &str) -> bool {
    std::path::Path::new(key)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tga"))
}

/// Decodes the image `raw` names, or why it is dropped: a path outside the
/// skin, a missing file, a type the validator does not decode (TabSRMM
/// takes PNG only, `tabsrmm.tsk:316`), a refused decode. A Clist_modern
/// (`png_only` false) `*.tga` atlas is read by the bounded [`tga`] decoder.
pub(crate) fn load(tree: &Tree, raw: &str, png_only: bool) -> Result<(String, RgbaImage), String> {
    let (key, bytes) = lookup(tree, raw).map_err(|e| e.to_string())?;
    if !png_only && is_tga(key) {
        let image = tga::decode(key, bytes).map_err(|e| e.to_string())?;
        return Ok((key.to_owned(), image));
    }
    match kind_of(bytes) {
        None => {
            return Err(format!(
                "{key}: not a bitmap the image validator decodes (PNG, GIF)"
            ))
        }
        // The validator refuses JPEG (plan E7.7.1 step 4); an importer
        // decodes only what the plan names for it (Winamp's RLE BMP).
        Some(ImageKind::Jpeg) => {
            return Err(format!("{key}: JPEG, which the image validator refuses"));
        }
        Some(k) if png_only && k != ImageKind::Png => {
            return Err(format!("{key}: {k:?}, but TabSRMM images are PNG only"));
        }
        Some(_) => {}
    }
    let image = bitmap::decode_unkeyed(key, bytes).map_err(|e| e.to_string())?;
    Ok((key.to_owned(), image))
}

/// The insets of a `w`×`h` 9-slice, refused when they leave no centre.
///
/// # Errors
/// [`ImportError::Image`] naming `source`.
pub(crate) fn insets(
    source: &str,
    (w, h): (u32, u32),
    [left, top, right, bottom]: [u32; 4],
) -> Result<Insets, ImportError> {
    let refuse = |detail: String| ImportError::Image {
        file: source.to_owned(),
        detail,
    };
    if left.saturating_add(right) >= w || top.saturating_add(bottom) >= h {
        return Err(refuse(format!(
            "9-slice insets {left}/{top}/{right}/{bottom} (left/top/right/bottom) are larger \
             than the {w}x{h} image"
        )));
    }
    let px = |v: u32| u16::try_from(v).map_err(|_| refuse(format!("inset {v} too large")));
    Ok(Insets {
        top: px(top)?,
        right: px(right)?,
        bottom: px(bottom)?,
        left: px(left)?,
    })
}

/// Scales every pixel's alpha by `num / den` (a constant alpha applied
/// after per-pixel alpha).
pub(crate) fn fade(image: &mut RgbaImage, num: u32, den: u32) {
    if num >= den {
        return;
    }
    for p in image.pixels_mut() {
        let a = u32::from(p.0[3]) * num / den;
        p.0[3] = u8::try_from(a).unwrap_or(u8::MAX);
    }
}

/// The slices as region slots: the first normal image of a slot sets it,
/// a state joins only a slot with a normal image of the same size and
/// insets; every other slice is named in `dropped`.
fn assemble(slices: Vec<Slice>, dropped: &mut Vec<String>) -> BTreeMap<&'static str, SlotImages> {
    let (normal, states): (Vec<_>, Vec<_>) = slices
        .into_iter()
        .partition(|s| s.state == RegionState::Normal);
    let mut regions: BTreeMap<&'static str, SlotImages> = BTreeMap::new();
    for s in normal {
        if regions.contains_key(s.slot) {
            dropped.push(format!("{}: {} is already set", s.source, s.slot));
            continue;
        }
        regions.insert(
            s.slot,
            SlotImages {
                insets: s.insets,
                images: BTreeMap::from([(RegionState::Normal, s.image)]),
                sources: BTreeMap::from([(RegionState::Normal, s.source)]),
            },
        );
    }
    for s in states {
        let why = match regions.get_mut(s.slot) {
            None => "no normal image for the slot",
            Some(r) if r.images.contains_key(&s.state) => "the state is already set",
            Some(r)
                if r.insets != s.insets
                    || r.images[&RegionState::Normal].dimensions() != s.image.dimensions() =>
            {
                "its size or insets differ from the normal image's"
            }
            Some(r) => {
                r.images.insert(s.state, s.image);
                r.sources.insert(s.state, s.source);
                continue;
            }
        };
        dropped.push(format!("{}: {} {:?}: {why}", s.source, s.slot, s.state));
    }
    regions
}

/// Reads `tree` (an unpacked Miranda NG skin directory).
///
/// # Errors
/// Not exactly one `.tsk` or `.msf`, a malformed skin file, a bad colour,
/// a 9-slice whose insets leave no centre.
pub fn read(tree: &Tree) -> Result<Read, ImportError> {
    let engine_of = |path: &str| {
        let ext = std::path::Path::new(path).extension()?.to_str()?;
        if ext.eq_ignore_ascii_case("tsk") {
            Some(Engine::TabSrmm)
        } else if ext.eq_ignore_ascii_case("msf") {
            Some(Engine::ClistModern)
        } else {
            None
        }
    };
    let skins: Vec<(&str, Engine)> = tree
        .keys()
        .filter_map(|k| Some((k.as_str(), engine_of(k)?)))
        .collect();
    let [(file, engine)] = skins[..] else {
        return Err(ImportError::Format {
            file: "skin directory".to_owned(),
            detail: format!(
                "{} skin files (.tsk or .msf); exactly one is read",
                skins.len()
            ),
        });
    };
    let ini = ini::parse(file, &tree[file])?;
    let parsed = match engine {
        Engine::TabSrmm => tabsrmm::parse(tree, file, &ini)?,
        Engine::ClistModern => clist::parse(tree, file, &ini)?,
    };
    let mut dropped = parsed.dropped;
    let regions = assemble(parsed.slices, &mut dropped);
    let unread = tree
        .keys()
        .filter(|k| k.as_str() != file && !parsed.files.contains(*k))
        .cloned()
        .collect();
    Ok(Read {
        engine,
        skin_file: file.to_owned(),
        info: parsed.info,
        colours: parsed.colours,
        tokens: parsed.tokens,
        regions,
        dropped,
        unread,
        repeated: ini
            .repeated
            .iter()
            .map(|r| format!("{file}: {r}"))
            .collect(),
    })
}

/// The package path of a slot state's image.
fn image_path(slot: &str, state: RegionState) -> String {
    let slot = slot.replace('.', "-");
    if state == RegionState::Normal {
        format!("assets/{slot}.png")
    } else {
        format!(
            "assets/{slot}-{}.png",
            format!("{state:?}").to_ascii_lowercase()
        )
    }
}

fn manifest(tree: &Tree, id: &str, read: &Read, source: &str, base: SkinBase) -> SkinManifest {
    let regions = read
        .regions
        .iter()
        .map(|(slot, r)| {
            let states = r
                .images
                .keys()
                .filter(|s| **s != RegionState::Normal)
                .map(|s| (*s, image_path(slot, *s)))
                .collect();
            let spec = RegionSpec {
                image: image_path(slot, RegionState::Normal),
                fit: RegionFit::NineSlice(r.insets),
                states,
                tint: None,
                mask: None,
                text_backing: TEXT_BACKINGS
                    .iter()
                    .find(|(s, _)| s == slot)
                    .map(|(_, t)| (*t).to_owned()),
            };
            ((*slot).to_owned(), spec)
        })
        .collect();
    let name = read
        .info
        .get("Name")
        .cloned()
        .unwrap_or_else(|| read.skin_file.clone());
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name: name.chars().take(64).collect(),
        description: "Imported from a Miranda NG skin; see the NOTICE beside the package."
            .to_owned(),
        license: SkinLicense::Text(
            "Third-party Miranda NG skin art under its author's terms. Not covered by Rekindle's \
             MIT licence; provenance and every change are in the NOTICE beside the package."
                .to_owned(),
        ),
        source: Some(SkinSource {
            format: MIRANDA_FORMAT.to_owned(),
            hash: source.to_owned(),
        }),
        // A Clist_modern `[Skin_Description_Section]` states `Author` and
        // `URL` (`clist_modern_skin.msf:4-6`); a tabSRMM `.tsk` states none.
        credits: Some(credits::build(
            tree,
            &read
                .info
                .get("Author")
                .cloned()
                .into_iter()
                .collect::<Vec<_>>(),
            &[],
            read.info.get("URL").map(String::as_str),
            None,
        )),
        author: None,
        signature: None,
        variants: vec![SkinVariant {
            base,
            contrast: SkinContrast::Standard,
            tokens: TOKENS.to_owned(),
        }],
        regions,
        icons: BTreeMap::new(),
        sounds: BTreeMap::new(),
    }
}

/// Imports `tree` as the package `id`: a passing package, or a draft
/// naming every failing pair.
///
/// # Errors
/// What [`read`] refuses, an image that does not encode.
pub fn import(tree: &Tree, id: &str) -> Result<Outcome, ImportError> {
    let read = read(tree)?;
    let source = tree_hash(tree);
    let (skin_base, base) = base_of(&read.tokens);
    let manifest_json = serde_json::to_vec(&manifest(tree, id, &read, &source, skin_base))
        .map_err(|e| ImportError::Format {
            file: MANIFEST.to_owned(),
            detail: e.to_string(),
        })?;
    let mut files = BTreeMap::from([(MANIFEST.to_owned(), manifest_json)]);
    for (slot, r) in &read.regions {
        for (state, img) in &r.images {
            let path = image_path(slot, *state);
            let png = encode_png(&DynamicImage::ImageRgba8(img.clone())).map_err(|e| {
                ImportError::Image {
                    file: path.clone(),
                    detail: e.to_string(),
                }
            })?;
            files.insert(path, png);
        }
    }
    let build = |tokens: &BTreeMap<String, TokenValue>| {
        let mut files = files.clone();
        files.insert(
            TOKENS.to_owned(),
            token_file(TOKENS, tokens).map_err(ImportError::Refused)?,
        );
        zip(&files).map_err(ImportError::Refused)
    };
    conclude(read.tokens.clone(), base, build, |changes| {
        notice::render(id, &read, &source, changes)
    })
}

#[cfg(test)]
mod tests;
