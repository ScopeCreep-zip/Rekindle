//! The Xfire INI-era importer (plan E7.7.3 item 2, "a delta on 1"), read
//! from DoiDD's shape (`.claude/plans/skins/raw/a/xfire-doidd/`; format:
//! `skins/research/a-legacy-formats.md` §1.3, d1 §3.1–§3.3).
//!
//! * Three files are read, each by the bounded [`ini`] reader and nothing
//!   else: `version.ini` (`[Version] Version=1`, `LongName`), the skin
//!   file `<LongName>.ini` (`[MainWindow]`), and the theme its
//!   `ColorGroupTheme=` names (`<name>.ini`, section `[default]`). Every
//!   other file (chat, info, in-game skins, `strings.ini`, `Thumbs.db`,
//!   `.psd`) is named in the `NOTICE` and never decoded.
//! * Colours: `ColorGroup.<Name>=RGBA(…)` by [`colours::NAMES`], parsed and
//!   targeted as in item 1 (a non-zero alpha refuses; absolute on a token,
//!   a signed shift on images, otherwise recorded).
//! * The window frame: the skin file's `[Top]` (`topleft`, `top`,
//!   `topright`), `[Left]`/`[Right]` (`middle`) and `[Bottom]`
//!   (`bottomleft`, `bottom`, `bottomright`) are the eight outer pieces of
//!   `window.frame`, `nine-slice`; the middle is transparent (the INI era
//!   draws the window's content there, not a piece). Each state prefix
//!   ([`STATES`]: `Graphics` normal, `Disabled` window-inactive, `Hover`,
//!   `BtnDown` down) with all eight pieces becomes that state's image, each
//!   piece shifted by its section's `<State>.ColorGroup` (DoiDD hover +30,
//!   pressed −30) and keyed (`#FF00FF` → alpha), as item 1's bitmaps.
//! * Window layout, triggers, text styles, other images and sounds are
//!   dropped and counted in the `NOTICE`; the package goes through
//!   [`super::conclude`] (a draft names every failing pair).

pub mod colours;
pub mod ini;
mod notice;

use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::{encode_png, DynamicImage, RgbaImage};
use rekindle_types::dto::skin::{
    Insets, RegionFit, RegionSpec, RegionState, SkinBase, SkinContrast, SkinLicense, SkinManifest,
    SkinSource, SkinVariant, TokenValue, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

use super::xfire_xml::colours::{absolute, parse_rgba, target_in, Target};
use super::xfire_xml::{bitmap, ColourRow};
use super::{base_of, conclude, tree_hash, walk, ImportError, Outcome, Tree};
use crate::output::{token_file, zip};
use crate::package::MANIFEST;
use ini::{Ini, Section};

/// The `source.format` of an imported package.
pub const XFIRE_INI_FORMAT: &str = "xfire-ini";
/// The file naming the skin.
pub const VERSION_FILE: &str = "version.ini";
const TOKENS: &str = "tokens.json";

/// The image state prefixes and the region state each becomes.
pub const STATES: [(&str, RegionState); 4] = [
    ("Graphics", RegionState::Normal),
    ("Disabled", RegionState::WindowInactive),
    ("Hover", RegionState::Hover),
    ("BtnDown", RegionState::Down),
];

/// The frame's pieces in [`bitmap::NINE`] order: (section, key); `None` is
/// the transparent middle.
pub const FRAME: [Option<(&str, &str)>; 9] = [
    Some(("Top", "topleft")),
    Some(("Top", "top")),
    Some(("Top", "topright")),
    Some(("Left", "middle")),
    None,
    Some(("Right", "middle")),
    Some(("Bottom", "bottomleft")),
    Some(("Bottom", "bottom")),
    Some(("Bottom", "bottomright")),
];

/// The composed window frame: its insets and one image per complete state.
#[derive(Debug)]
pub struct Frame {
    pub insets: Insets,
    pub images: BTreeMap<RegionState, RgbaImage>,
    /// The image files decoded for it.
    pub files: BTreeSet<String>,
    /// A state that was not emitted, and why.
    pub skipped: Vec<(RegionState, String)>,
}

/// What the importer read, before the package is built.
#[derive(Debug)]
pub struct Read {
    pub info: BTreeMap<&'static str, String>,
    pub colours: Vec<ColourRow>,
    pub tokens: BTreeMap<String, TokenValue>,
    pub frame: Option<Frame>,
    pub dropped: BTreeMap<&'static str, usize>,
    /// Files of the tree that were not read.
    pub unread: Vec<String>,
    /// Repeated keys not read (the first occurrence wins), per file.
    pub repeated: Vec<String>,
}

/// `raw` (a Windows path) as a tree key: `\` → `/`, relative, no `.`/`..`
/// segment, matched exactly, else ASCII-case-insensitively (Windows).
pub(crate) fn lookup<'t>(tree: &'t Tree, raw: &str) -> Result<(&'t str, &'t [u8]), ImportError> {
    let path = raw.replace('\\', "/");
    if path.contains(':')
        || path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(ImportError::Format {
            file: raw.to_owned(),
            detail: "not a relative path inside the skin".to_owned(),
        });
    }
    tree.get_key_value(&path)
        .or_else(|| tree.iter().find(|(k, _)| k.eq_ignore_ascii_case(&path)))
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .ok_or(ImportError::MissingFile { file: path })
}

fn parse_file<'t>(tree: &'t Tree, name: &str) -> Result<(&'t str, Ini), ImportError> {
    let (key, bytes) = lookup(tree, name)?;
    Ok((key, ini::parse(key, bytes)?))
}

fn section<'i>(ini: &'i Ini, file: &str, name: &str) -> Result<&'i Section, ImportError> {
    ini.section(name).ok_or_else(|| ImportError::Format {
        file: file.to_owned(),
        detail: format!("no [{name}]"),
    })
}

/// A file name from a value: one path segment, no separator.
fn file_name(file: &str, key: &str, value: &str) -> Result<String, ImportError> {
    if value.is_empty() || value.len() > 64 || value.contains(['/', '\\', ':']) || value == ".." {
        return Err(ImportError::Format {
            file: file.to_owned(),
            detail: format!("{key}={value} is not a file name"),
        });
    }
    Ok(format!("{value}.ini"))
}

type Theme = (
    Vec<ColourRow>,
    BTreeMap<String, TokenValue>,
    BTreeMap<String, String>,
);

/// The theme's colours, the tokens they set, and its `Graphics.<ALIAS>`
/// image aliases; every other key counted in `dropped`.
fn read_theme(
    theme: &Section,
    dropped: &mut BTreeMap<&'static str, usize>,
) -> Result<Theme, ImportError> {
    let (mut rows, mut tokens, mut aliases) = (Vec::new(), BTreeMap::new(), BTreeMap::new());
    for e in &theme.entries {
        if let Some(name) = e.key.strip_prefix("ColorGroup.") {
            let value = parse_rgba(name, &e.value)?;
            let t = target_in(colours::NAMES, name);
            if let Target::Token(id) = t {
                tokens.insert(id.to_owned(), walk::color(absolute(name, &e.value, value)?));
            }
            rows.push(ColourRow {
                name: name.to_owned(),
                value,
                target: t,
            });
        } else if let Some(alias) = e.key.strip_prefix("Graphics.") {
            aliases.insert(alias.to_ascii_uppercase(), e.value.clone());
        } else {
            *dropped.entry("theme keys (sounds, other)").or_default() += 1;
        }
    }
    Ok((rows, tokens, aliases))
}

/// One state's pieces (decoded only when all eight are named), shifted and
/// keyed, each decoded file added to `files`; or the keys it lacks.
fn state_pieces(
    tree: &Tree,
    skin: &Ini,
    prefix: &str,
    colours: &[ColourRow],
    aliases: &BTreeMap<String, String>,
    files: &mut BTreeSet<String>,
) -> Result<Result<[RgbaImage; 9], Vec<String>>, ImportError> {
    let mut missing = Vec::new();
    let mut named = Vec::with_capacity(8);
    for (sec, key) in FRAME.into_iter().flatten() {
        let s = skin.section(sec);
        match s.and_then(|s| s.get(&format!("{prefix}.{key}"))) {
            Some(value) => named.push((s, value)),
            None => missing.push(format!("[{sec}] {prefix}.{key}")),
        }
    }
    if !missing.is_empty() {
        return Ok(Err(missing));
    }
    let mut pieces: Vec<RgbaImage> = Vec::with_capacity(9);
    for (s, value) in named {
        let path = if value.contains('.') {
            value
        } else {
            aliases
                .get(&value.to_ascii_uppercase())
                .map(String::as_str)
                .ok_or_else(|| ImportError::MissingFile {
                    file: format!("Graphics.{value}"),
                })?
        };
        let (file, bytes) = lookup(tree, path)?;
        files.insert(file.to_owned());
        let mut img = bitmap::decode(file, bytes)?;
        // An undefined group is the identity shift (d1 §3.1).
        let by = s
            .and_then(|s| s.get(&format!("{prefix}.ColorGroup")))
            .and_then(|c| colours.iter().find(|r| r.name == c))
            .map_or([0; 3], |r| r.value);
        bitmap::shift(&mut img, by);
        pieces.push(img);
    }
    // The transparent middle, between the left and right pieces.
    pieces.insert(4, RgbaImage::new(1, 1));
    Ok(Ok(pieces.try_into().map_err(|_| {
        ImportError::MissingFile {
            file: "frame pieces".to_owned(),
        }
    })?))
}

fn read_frame(
    tree: &Tree,
    skin: &Ini,
    colours: &[ColourRow],
    aliases: &BTreeMap<String, String>,
) -> Result<Option<Frame>, ImportError> {
    let mut frame: Option<Frame> = None;
    for (prefix, state) in STATES {
        let mut files = BTreeSet::new();
        let pieces = match state_pieces(tree, skin, prefix, colours, aliases, &mut files)? {
            Ok(p) => p,
            Err(missing) => {
                if let Some(f) = frame.as_mut().filter(|_| missing.len() < 8) {
                    f.skipped
                        .push((state, format!("missing {}", missing.join(", "))));
                }
                continue;
            }
        };
        let (img, insets) = bitmap::compose("window frame", &pieces)?;
        match frame.as_mut() {
            None if state == RegionState::Normal => {
                frame = Some(Frame {
                    insets,
                    images: BTreeMap::from([(state, img)]),
                    files,
                    skipped: Vec::new(),
                });
            }
            None => {}
            Some(f) if f.insets == insets => {
                f.images.insert(state, img);
                f.files.extend(files);
            }
            Some(f) => f.skipped.push((
                state,
                "its pieces differ in size from the normal state's".to_owned(),
            )),
        }
    }
    Ok(frame)
}

/// Every key of the skin file the importer does not use, by kind.
fn count_dropped(skin: &Ini, frame: Option<&Frame>, dropped: &mut BTreeMap<&'static str, usize>) {
    let used = |sec: &str, key: &str| {
        frame.is_some_and(|f| {
            STATES
                .iter()
                .filter(|(_, s)| f.images.contains_key(s))
                .any(|(p, _)| {
                    FRAME.iter().flatten().any(|(fs, fk)| {
                        sec.eq_ignore_ascii_case(fs)
                            && (key.eq_ignore_ascii_case(&format!("{p}.{fk}"))
                                || key.eq_ignore_ascii_case(&format!("{p}.ColorGroup")))
                    })
                })
        })
    };
    for s in &skin.sections {
        for e in s.entries.iter().filter(|e| !used(&s.name, &e.key)) {
            let k = e.key.as_str();
            let kind =
                if k.starts_with("Window") && k[6..].starts_with(|c: char| c.is_ascii_digit()) {
                    "window layout"
                } else if k.starts_with("Action.") {
                    "triggers"
                } else if k.starts_with("Text.") {
                    "text styles"
                } else if STATES
                    .iter()
                    .any(|(p, _)| k.strip_prefix(p).is_some_and(|r| r.starts_with('.')))
                {
                    "images outside the window frame"
                } else if s.name.eq_ignore_ascii_case("MainWindow") && k.ends_with("Skin") {
                    "other skin files (chat, group chat, in-game)"
                } else {
                    "other keys (classes, sizes, cursors, sounds)"
                };
            *dropped.entry(kind).or_default() += 1;
        }
    }
}

/// Reads `tree` (an unpacked Xfire INI-era skin).
///
/// # Errors
/// A missing or malformed file, a non-zero alpha, a bad colour or image.
pub fn read(tree: &Tree) -> Result<Read, ImportError> {
    let (version_key, version) = parse_file(tree, VERSION_FILE)?;
    let v = section(&version, version_key, "Version")?;
    if v.get("Version") != Some("1") {
        return Err(ImportError::Format {
            file: version_key.to_owned(),
            detail: "Version is not 1".to_owned(),
        });
    }
    let long = v.get("LongName").unwrap_or_default().to_owned();
    let (skin_key, skin) = parse_file(tree, &file_name(version_key, "LongName", &long)?)?;
    let main = section(&skin, skin_key, "MainWindow")?;
    let theme_name = main.get("ColorGroupTheme").unwrap_or_default();
    let (theme_key, theme) =
        parse_file(tree, &file_name(skin_key, "ColorGroupTheme", theme_name)?)?;
    let mut dropped = BTreeMap::new();
    let (colours, tokens, aliases) =
        read_theme(section(&theme, theme_key, "default")?, &mut dropped)?;
    let frame = read_frame(tree, &skin, &colours, &aliases)?;
    count_dropped(&skin, frame.as_ref(), &mut dropped);
    let read_files = [version_key, skin_key, theme_key];
    let repeated = [
        (version_key, &version),
        (skin_key, &skin),
        (theme_key, &theme),
    ]
    .into_iter()
    .flat_map(|(f, i)| i.repeated.iter().map(move |r| format!("{f}: {r}")))
    .collect();
    let decoded = frame.as_ref().map(|f| &f.files);
    let unread = tree
        .keys()
        .filter(|k| !read_files.contains(&k.as_str()) && !decoded.is_some_and(|d| d.contains(*k)))
        .cloned()
        .collect();
    let info = BTreeMap::from([
        ("LongName", long),
        ("skin file", skin_key.to_owned()),
        ("theme file", theme_key.to_owned()),
    ]);
    Ok(Read {
        info,
        colours,
        tokens,
        frame,
        dropped,
        unread,
        repeated,
    })
}

/// The package path of a frame state's image.
fn frame_image(state: RegionState) -> String {
    let name = match state {
        RegionState::Normal => "window-frame",
        RegionState::WindowInactive => "window-frame-inactive",
        RegionState::Hover => "window-frame-hover",
        RegionState::Down => "window-frame-down",
        _ => "window-frame-other",
    };
    format!("assets/{name}.png")
}

fn manifest(id: &str, read: &Read, source: &str, base: SkinBase) -> SkinManifest {
    let mut regions = BTreeMap::new();
    if let Some(f) = &read.frame {
        let states = f
            .images
            .keys()
            .filter(|s| **s != RegionState::Normal)
            .map(|s| (*s, frame_image(*s)))
            .collect();
        regions.insert(
            "window.frame".to_owned(),
            RegionSpec {
                image: frame_image(RegionState::Normal),
                fit: RegionFit::NineSlice(f.insets),
                states,
                tint: None,
                mask: None,
                text_backing: None,
            },
        );
    }
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name: read
            .info
            .get("LongName")
            .map_or_else(|| id.to_owned(), |n| n.chars().take(64).collect()),
        description: "Imported from an Xfire INI-era skin; see the NOTICE beside the package."
            .to_owned(),
        license: SkinLicense::Text(
            "Third-party Xfire skin art, published without a licence. Not covered by Rekindle's \
             MIT licence; provenance and every change are in the NOTICE beside the package."
                .to_owned(),
        ),
        source: Some(SkinSource {
            format: XFIRE_INI_FORMAT.to_owned(),
            hash: source.to_owned(),
        }),
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
    let manifest_json =
        serde_json::to_vec(&manifest(id, &read, &source, skin_base)).map_err(|e| {
            ImportError::Format {
                file: MANIFEST.to_owned(),
                detail: e.to_string(),
            }
        })?;
    let mut files = BTreeMap::from([(MANIFEST.to_owned(), manifest_json)]);
    for (state, img) in read.frame.iter().flat_map(|f| &f.images) {
        let path = frame_image(*state);
        let png =
            encode_png(&DynamicImage::ImageRgba8(img.clone())).map_err(|e| ImportError::Image {
                file: path.clone(),
                detail: e.to_string(),
            })?;
        files.insert(path, png);
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
