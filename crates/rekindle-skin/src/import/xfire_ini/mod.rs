//! The Xfire INI-era importer (plan E7.7.3 item 2, "a delta on 1"), read
//! from DoiDD's shape (`.claude/plans/skins/raw/a/xfire-doidd/`; format:
//! `skins/research/a-legacy-formats.md` §1.3, d1 §3.1–§3.3).
//!
//! * Three files are read, each by the bounded [`ini`] reader and nothing
//!   else: `version.ini` (`[Version] Version=1`, `LongName`), the skin
//!   file `<folder>.ini` (`[MainWindow]`), and the theme its
//!   `ColorGroupTheme=` names (`<name>.ini`, section `[default]`), and the
//!   chat and group chat skin files its `ChatSkin=`/`GroupChatSkin=` name,
//!   for their surfaces only (plan E7.7.7 S2; read when one fills a slot).
//!   Every other file (info, in-game skins, `strings.ini`, `Thumbs.db`,
//!   `.psd`) is named in the `NOTICE` and never decoded.
//! * The skin file is named by the skin's folder, never by `LongName`,
//!   which is only the name shown: in all 45 INI archives of the research
//!   item (`.claude/plans/skins/raw/xfire-online/ia_XFireSkins/`) the
//!   folder holding `version.ini` holds `<folder>.ini` (case aside:
//!   `Xfireaqua/Xfireaqua.ini`), while `<LongName>.ini` is absent in 13,
//!   Xfire's own partner skin among them (`Charger/Charger.ini`,
//!   `LongName=Dodge Charger`; `Xbox360/Xbox360.ini`, `LongName=Xbox 360`);
//!   the XML era's `Info.xml` likewise carries `Name` and `LongName` apart
//!   (Carbon Fiber's `LongName` is `James Fiber`).
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
//! * Window layout, text styles, other images and sounds are
//!   dropped and counted in the `NOTICE`; the package goes through
//!   [`super::conclude`] (a draft names every failing pair).

pub mod chat;
pub mod colours;
pub mod controls;
pub mod frame;
pub mod ini;
mod menu_ground;
mod notice;
pub mod title;

use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::{encode_png, DynamicImage};
use rekindle_types::dto::skin::{
    RegionFit, RegionSpec, RegionState, SkinBase, SkinContrast, SkinLicense, SkinManifest,
    SkinSource, SkinVariant, SkinWindow, TokenValue, WindowLayout, SKIN_FORMAT_MAJOR,
    SKIN_FORMAT_PREFIX,
};

use super::credit::{Authorship, XfireCredit};
use super::credits;
use super::xfire_xml::colours::{absolute, parse_rgba, target_in, Target};
use super::xfire_xml::controls::StateTally;
use super::xfire_xml::refs::{self, Ref};
use super::xfire_xml::title as xtitle;
use super::xfire_xml::{bitmap, ColourRow};
use super::{base_of, conclude, tree_hash, walk, Control, ImportError, Outcome, Tree};
use crate::output::{token_file, zip};
use crate::package::MANIFEST;
use frame::read_frame;
pub use frame::{Frame, Shape, FRAME, MIDDLE_FRAME, OUTER_FRAME, SHAPES};
use ini::{Ini, Section};

/// The `source.format` of an imported package.
pub const XFIRE_INI_FORMAT: &str = "xfire-ini";
/// The file naming the skin.
pub const VERSION_FILE: &str = "version.ini";
const TOKENS: &str = "tokens.json";
/// The longest `Version` recorded, in scalars.
pub const MAX_REVISION: usize = 32;

/// The image state prefixes and the region state each becomes.
pub const STATES: [(&str, RegionState); 4] = [
    ("Graphics", RegionState::Normal),
    ("Disabled", RegionState::WindowInactive),
    ("Hover", RegionState::Hover),
    ("BtnDown", RegionState::Down),
];

/// What the importer read, before the package is built.
#[derive(Debug)]
pub struct Read {
    pub info: BTreeMap<&'static str, String>,
    pub colours: Vec<ColourRow>,
    pub tokens: BTreeMap<String, TokenValue>,
    pub frame: Option<Frame>,
    /// The buddy list's layout from the frame, with the title strip the
    /// skin file lays ([`title`]), and its package images.
    pub main: Option<(WindowLayout, BTreeMap<String, Vec<u8>>)>,
    /// The chat window its `ChatSkin=` file lays ([`chat`]), if any.
    pub chat: Option<chat::Built>,
    pub dropped: BTreeMap<&'static str, usize>,
    /// Files of the tree that were not read.
    pub unread: Vec<String>,
    /// Repeated keys not read (the first occurrence wins), per file.
    pub repeated: Vec<String>,
    /// The authors `version.ini`'s comments name ([`stated_authors`]).
    pub authors: Vec<String>,
    /// The painted control slots ([`controls`]) and what they kept.
    pub controls: Vec<Control>,
    /// The mean of the frame art under the menu text ([`menu_ground`]).
    pub menu_ground: Option<[u8; 3]>,
    pub tally: StateTally,
    /// The colour roles the sections name ([`refs`]).
    pub refs: Vec<refs::RefsApplied>,
}

/// The names `comments` credit: the text after the last ` by ` of each
/// comment that has one (`; Alloy Xfire Skin by Gunowar`, Xglow's
/// `;Designed by [S.C.o.D]GrAsMaN` and `;Coded by Avicus|PSK`), in order,
/// each once. The manifest bounds and checks them like `name`.
#[must_use]
pub fn stated_authors(comments: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in comments {
        let lower = c.to_ascii_lowercase();
        // ASCII lowering keeps every byte offset.
        let start = match lower.rfind(" by ") {
            Some(at) => at + 4,
            None if lower.starts_with("by ") => 3,
            None => continue,
        };
        let name = c[start..].trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
    }
    out
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

pub(super) fn parse_file<'t>(tree: &'t Tree, name: &str) -> Result<(&'t str, Ini), ImportError> {
    let (key, bytes) = lookup(tree, name)?;
    Ok((key, ini::parse(key, bytes)?))
}

/// The `<Popups>` node of every popup file a section of `skin` launches
/// (`Skin.Name=`, [`title::popup`]) and the files read: the first launcher
/// of a name places it (repeated keys read the first, [`ini`]). A launched
/// file the pack lacks, or one without its root section, refuses the pack.
pub(super) fn popups<'t>(
    tree: &'t Tree,
    skin_key: &str,
    skin: &Ini,
) -> Result<(Vec<crate::import::tree::Node>, Vec<&'t str>), ImportError> {
    let mut seen = BTreeSet::new();
    let (mut nodes, mut keys) = (Vec::new(), Vec::new());
    for s in &skin.sections {
        let Some(name) = s.get("Skin.Name").or(s.get("Skin")).map(str::trim) else {
            continue;
        };
        if !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        let (key, file) = parse_file(tree, &file_name(skin_key, "Skin.Name", name)?)?;
        nodes.push(
            title::popup(s, name, &file).ok_or_else(|| ImportError::Format {
                file: key.to_owned(),
                detail: format!("no [{name}]"),
            })?,
        );
        keys.push(key);
    }
    Ok((nodes, keys))
}

fn section<'i>(ini: &'i Ini, file: &str, name: &str) -> Result<&'i Section, ImportError> {
    ini.section(name).ok_or_else(|| ImportError::Format {
        file: file.to_owned(),
        detail: format!("no [{name}]"),
    })
}

/// A file name from a value: one path segment, no separator.
pub(super) fn file_name(file: &str, key: &str, value: &str) -> Result<String, ImportError> {
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

/// An image value: a path (it holds a `.`), else a theme `Graphics.<NAME>`
/// alias, read through [`lookup`].
pub(crate) fn resolve<'t>(
    tree: &'t Tree,
    value: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<(&'t str, &'t [u8]), ImportError> {
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
    lookup(tree, path)
}

/// Every key of the skin file the importer does not use, by kind.
pub(super) fn count_dropped(
    skin: &Ini,
    (frame, control_sections): (Option<&Frame>, &BTreeSet<String>),
    dropped: &mut BTreeMap<&'static str, usize>,
) {
    let used = |sec: &str, key: &str| {
        title::is_trigger_key(key)
            || control_sections.contains(&sec.to_ascii_lowercase()) && controls::is_read(key)
            || frame.is_some_and(|f| {
                STATES
                    .iter()
                    .filter(|(_, s)| f.images.contains_key(s))
                    .any(|(p, _)| {
                        f.shape.iter().flatten().any(|(fs, fk)| {
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
                } else if k.starts_with("Text.") {
                    "text styles"
                } else if STATES
                    .iter()
                    .any(|(p, _)| k.strip_prefix(p).is_some_and(|r| r.starts_with('.')))
                {
                    "images with no region slot"
                } else if s.name.eq_ignore_ascii_case("MainWindow") && k.ends_with("Skin") {
                    "other skin files (chat, group chat, in-game)"
                } else {
                    "other keys (classes, sizes, cursors, sounds)"
                };
            *dropped.entry(kind).or_default() += 1;
        }
    }
}

/// Reads `tree` (an unpacked Xfire INI-era skin) from its folder `folder`
/// (the directory holding `version.ini`, whose name names the skin file).
///
/// # Errors
/// A missing or malformed file, a non-zero alpha, a bad colour or image.
pub fn read(tree: &Tree, folder: &str) -> Result<Read, ImportError> {
    let (version_key, version) = parse_file(tree, VERSION_FILE)?;
    let v = section(&version, version_key, "Version")?;
    // `Version` is the skin's own revision, never a format selector: across
    // the 43 INI skins of the 2026-11-17 research index it is `1` (37), `2`
    // (3, Combat among them), `01.11.2005`, `1.54` and `RC 2`, all one
    // layout. It is recorded, bounded, in the NOTICE.
    let revision: String = v
        .get("Version")
        .unwrap_or_default()
        .chars()
        .take(MAX_REVISION)
        .collect();
    let long = v.get("LongName").unwrap_or_default().to_owned();
    let (skin_key, skin) = parse_file(tree, &file_name(version_key, "the skin folder", folder)?)?;
    let main = section(&skin, skin_key, "MainWindow")?;
    let theme_name = main.get("ColorGroupTheme").unwrap_or_default();
    let (theme_key, theme) =
        parse_file(tree, &file_name(skin_key, "ColorGroupTheme", theme_name)?)?;
    let mut dropped = BTreeMap::new();
    let (colours, mut tokens, aliases) =
        read_theme(section(&theme, theme_key, "default")?, &mut dropped)?;
    let frame = read_frame(tree, &skin, &colours, &aliases)?;
    let images = controls::Images {
        tree,
        colours: &colours,
        aliases: &aliases,
        files: std::cell::RefCell::default(),
    };
    let (mut controls, mut tally, control_sections) = controls::read(&skin, &images)?;
    let mut main_layout = frame.as_ref().map(frame::main_layout).transpose()?;
    let (popup_nodes, popup_keys) = popups(tree, skin_key, &skin)?;
    let decode = |value: &str, group: Option<&str>| {
        let (file, bytes) = resolve(tree, value, &aliases)?;
        images.files.borrow_mut().insert(file.to_owned());
        let mut img = bitmap::decode(file, bytes)?;
        let by = group
            .and_then(|c| colours.iter().find(|r| r.name == c))
            .map_or([0; 3], |r| r.value);
        bitmap::shift(&mut img, by);
        Ok(img)
    };
    if let (Some(built), Some((layout, components))) =
        (main_layout.as_mut(), title::tree(&skin, "MainWindow"))
    {
        let mut all = vec![components];
        all.extend(popup_nodes);
        let components = all.as_slice();
        xtitle::add(
            SkinWindow::Main,
            &layout,
            components,
            &colours,
            decode,
            built,
        )?;
        // The INI frame is the window section's own pieces: its box.
        crate::skin_tree::import::add(SkinWindow::Main, &layout, components, decode, &[], built)?;
    }
    count_dropped(&skin, (frame.as_ref(), &control_sections), &mut dropped);
    let mut read_files = vec![version_key, skin_key, theme_key];
    read_files.extend(popup_keys);
    let mut found = Vec::new();
    ini_refs(&skin, &mut found);
    let chat_window = chat::read_files(
        tree,
        (skin_key, main),
        &images,
        &decode,
        &mut chat::Sink {
            controls: &mut controls,
            tally: &mut tally,
            read_files: &mut read_files,
            found: &mut found,
            dropped: &mut dropped,
        },
    )?;
    let refs = refs::apply(&found, &colours, &mut tokens);
    let control_files = images.files.into_inner();
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
        .filter(|k| {
            !read_files.contains(&k.as_str())
                && !decoded.is_some_and(|d| d.contains(*k))
                && !control_files.contains(*k)
        })
        .cloned()
        .collect();
    let info = BTreeMap::from([
        ("LongName", long),
        ("Version (the skin's own revision)", revision),
        ("skin file", skin_key.to_owned()),
        ("theme file", theme_key.to_owned()),
    ]);
    let menu_ground = menu_ground::of(&skin, frame.as_ref());
    Ok(Read {
        info,
        colours,
        tokens,
        frame,
        main: main_layout,
        chat: chat_window,
        dropped,
        unread,
        repeated,
        authors: stated_authors(&version.comments),
        menu_ground,
        controls,
        tally,
        refs,
    })
}

/// Every colour reference of `ini` in file order ([`refs`]): a section's
/// `ColorGroup=` and its `Text.<attr>=` keys (`Graphics.ColorGroup` is an
/// image shift, never a role).
pub(super) fn ini_refs(ini: &Ini, out: &mut Vec<Ref>) {
    for s in &ini.sections {
        for e in &s.entries {
            let key = e.key.to_ascii_lowercase();
            if key == "colorgroup" || key.starts_with("text.") {
                out.push(Ref {
                    component: s.name.clone(),
                    attr: key,
                    colour: e.value.clone(),
                });
            }
        }
    }
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

fn manifest(
    id: &str,
    read: &Read,
    (source, tree): (&str, &Tree),
    base: SkinBase,
    credit: Option<&XfireCredit>,
    windows: BTreeMap<SkinWindow, WindowLayout>,
) -> SkinManifest {
    let authorship = credit.map_or(Authorship::ThirdParty, Authorship::XfireByCredit);
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
    for c in &read.controls {
        regions.insert(c.region.to_owned(), c.spec());
    }
    SkinManifest {
        text: BTreeMap::new(),
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name: read
            .info
            .get("LongName")
            .map_or_else(|| id.to_owned(), |n| n.chars().take(64).collect()),
        description: "Imported from an Xfire INI-era skin; see the NOTICE beside the package."
            .to_owned(),
        license: SkinLicense::Text(authorship.manifest_text()),
        source: Some(SkinSource {
            format: XFIRE_INI_FORMAT.to_owned(),
            hash: source.to_owned(),
        }),
        // An INI skin names its authors only in `version.ini` comments, and
        // no site; an outside credit names Xfire and its page.
        credits: Some(credits::build(tree, &read.authors, &[], None, credit)),
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
        windows,
    }
}

/// Imports `tree`, the skin folder `folder`, as the package `id`: a
/// passing package, or a draft
/// naming every failing pair. `credit` is Xfire's authorship stated outside
/// the files (an INI skin carries no author); without it the pack is
/// third-party work.
///
/// # Errors
/// What [`read`] refuses, an image that does not encode.
pub fn import(
    tree: &Tree,
    folder: &str,
    id: &str,
    credit: Option<&XfireCredit>,
) -> Result<Outcome, ImportError> {
    let read = read(tree, folder)?;
    let source = tree_hash(tree);
    let (skin_base, base) = base_of(&read.tokens);
    let authorship = credit.map_or(Authorship::ThirdParty, Authorship::XfireByCredit);
    // The skin file is the main window's (`MainSkin`); its frame is the
    // buddy list's outline (plan SK0 S1).
    let windows = [
        (SkinWindow::Main, &read.main),
        (SkinWindow::Chat, &read.chat),
    ]
    .into_iter()
    .filter_map(|(w, b)| Some((w, b.as_ref()?.0.clone())))
    .collect();
    let manifest = manifest(id, &read, (&source, tree), skin_base, credit, windows);
    let manifest_json = serde_json::to_vec(&manifest).map_err(|e| ImportError::Format {
        file: MANIFEST.to_owned(),
        detail: e.to_string(),
    })?;
    let mut files = BTreeMap::from([(MANIFEST.to_owned(), manifest_json)]);
    for (_, tiles) in read.main.iter().chain(&read.chat) {
        files.extend(tiles.clone());
    }
    for (state, img) in read.frame.iter().flat_map(|f| &f.images) {
        let path = frame_image(*state);
        let png =
            encode_png(&DynamicImage::ImageRgba8(img.clone())).map_err(|e| ImportError::Image {
                file: path.clone(),
                detail: e.to_string(),
            })?;
        files.insert(path, png);
    }
    for c in &read.controls {
        files.extend(c.files()?);
    }
    let build = |tokens: &BTreeMap<String, TokenValue>| {
        let mut files = files.clone();
        files.insert(
            TOKENS.to_owned(),
            token_file(TOKENS, tokens).map_err(ImportError::Refused)?,
        );
        zip(&files).map_err(ImportError::Refused)
    };
    let tokens =
        super::dialog_roles::with_art(read.tokens.clone(), &read.controls, read.menu_ground);
    conclude(tokens, base, build, |changes| {
        notice::render(id, &read, &source, authorship, changes)
    })
}

#[cfg(test)]
mod controls_tests;
#[cfg(test)]
mod tests;
