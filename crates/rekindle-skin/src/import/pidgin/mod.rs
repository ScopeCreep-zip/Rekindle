//! The Pidgin 2.x theme importer (plan E7.7.3 item 4): the buddy-list
//! palette, status icons and sound events of one theme directory.
//!
//! A theme is a directory holding `purple/<type>/theme.xml` for any of the
//! types `blist`, `status-icon` and `sound` (Pidgin's `theme-manager.c`
//! and loaders, `skins/research/a-legacy-formats.md` §4.6), at the tree's
//! root or under one top-level folder. The tree is bounded by the legacy
//! limits (≤ [`MAX_ENTRIES`] files, ≤ [`MAX_UNCOMPRESSED_BYTES`] together);
//! each `theme.xml` goes through the bounded reader (no DTD, an entity
//! reference refuses the file). [`map`] says what each item becomes:
//! * colours to registry tokens (`#rgb`/`#rrggbb` only); fonts dropped;
//! * status icons to the status slots, each through
//!   [`crate::images::check`] (PNG/GIF only), re-encoded;
//! * sound events to cue ids, each through [`crate::sounds::check`]
//!   (≤ 5 s, ≤ 48 kHz, ≤ 2 channels), re-encoded to PCM16 WAV.
//!
//! Every item that is unknown, unmapped, not a plain relative path, absent,
//! or refused by its validator is dropped and named in the NOTICE; the rest
//! builds the package, which settles to [`Outcome::Passed`] or an
//! [`Outcome::Draft`] like every importer.

pub mod map;
mod notice;

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{
    SkinBase, SkinContrast, SkinError, SkinLicense, SkinManifest, SkinSource, SkinVariant,
    TokenValue, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

use super::tree::{self, Node};
use super::winamp::archive::MAX_UNCOMPRESSED_BYTES;
use super::{base_of, conclude, tree_hash, walk, ImportError, Outcome, Tree};
use crate::limits::MAX_ENTRIES;
use crate::output::{token_file, zip};
use crate::package::MANIFEST;
use crate::{images, sounds};
use map::Target;

/// The `source.format` of an imported package.
pub const PIDGIN_FORMAT: &str = "pidgin-theme";
const TOKENS: &str = "tokens.json";
const BLIST: &str = "blist";
const ICONS: &str = "status-icon";
const SOUND: &str = "sound";
const BLIST_TYPE: &str = "pidgin buddy list";
const FONT_DROPPED: &str =
    "dropped: a Pango font description; fonts resolve only against the bundled allow-list";

/// One item that did not enter the package, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub item: String,
    pub why: String,
}

/// One colour the theme defines and what it became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColourRow {
    pub source: String,
    pub value: [u8; 3],
    pub token: &'static str,
}

/// What the importer read, before the package is built.
#[derive(Debug, Default)]
pub struct Read {
    /// Each `theme.xml` read: its type and `name`.
    pub themes: Vec<(&'static str, String)>,
    pub colours: Vec<ColourRow>,
    pub tokens: BTreeMap<String, TokenValue>,
    /// Status slot → (package path, canonical bytes, source item).
    pub icons: BTreeMap<String, (String, Vec<u8>, String)>,
    /// Cue id → (package path, canonical WAV, source item).
    pub sounds: BTreeMap<String, (String, Vec<u8>, String)>,
    pub dropped: Vec<Dropped>,
}

impl Read {
    fn drop(&mut self, item: impl Into<String>, why: impl Into<String>) {
        self.dropped.push(Dropped {
            item: item.into(),
            why: why.into(),
        });
    }
}

fn refused(entry: &str, detail: impl Into<String>) -> ImportError {
    ImportError::Archive {
        entry: entry.to_owned(),
        detail: detail.into(),
    }
}

/// The legacy limits on the tree as a whole.
fn check_limits(tree: &Tree) -> Result<(), ImportError> {
    if tree.len() as u64 > MAX_ENTRIES {
        return Err(refused(
            "theme",
            format!("{} files is past {MAX_ENTRIES}", tree.len()),
        ));
    }
    let total: u64 = tree.values().map(|b| b.len() as u64).sum();
    if total > MAX_UNCOMPRESSED_BYTES {
        return Err(refused(
            "theme",
            format!("{total} bytes is past {MAX_UNCOMPRESSED_BYTES}"),
        ));
    }
    Ok(())
}

/// The folder holding `purple/`: the root, or one top-level folder.
fn theme_root(tree: &Tree) -> Result<String, ImportError> {
    let found = tree.keys().find_map(|k| {
        [BLIST, ICONS, SOUND].iter().find_map(|t| {
            let at = k.strip_suffix(&format!("purple/{t}/theme.xml"))?;
            (at.is_empty() || (at.ends_with('/') && at.matches('/').count() == 1))
                .then(|| at.to_owned())
        })
    });
    found.ok_or_else(|| ImportError::MissingFile {
        file: "purple/{blist,status-icon,sound}/theme.xml".to_owned(),
    })
}

/// `purple/<kind>/theme.xml` parsed, if the theme has that type.
fn theme(tree: &Tree, root: &str, kind: &str) -> Result<Option<(String, Node)>, ImportError> {
    let dir = format!("{root}purple/{kind}/");
    let file = format!("{dir}theme.xml");
    tree.get(&file)
        .map(|bytes| tree::parse(&file, bytes).map(|n| (dir, n)))
        .transpose()
}

/// The root's `name`, and its `type` when the loader requires one.
fn named(file: &str, node: &Node, required_type: Option<&str>) -> Result<String, ImportError> {
    let bad = |detail: &str| ImportError::Xml {
        file: file.to_owned(),
        detail: detail.to_owned(),
    };
    let name = node.attr("name").ok_or_else(|| bad("no name attribute"))?;
    if let Some(t) = required_type {
        if node.attr("type") != Some(t) {
            return Err(bad(&format!("the root's type is not \"{t}\"")));
        }
    }
    Ok(name.to_owned())
}

/// The first child at `path` (`a/b`), as Pidgin's `get_child` finds it.
fn at<'a>(node: &'a Node, path: &'a str) -> Option<&'a Node> {
    path.split('/')
        .try_fold(node, |n, part| n.children_named(part).next())
}

fn read_blist(read: &mut Read, node: &Node) {
    let mut fonts = Vec::new();
    for (path, attr, target) in map::COLOURS {
        let Some(el) = at(node, path) else { continue };
        if let Some(font) = el.attr("font").filter(|_| !fonts.contains(&path)) {
            fonts.push(path);
            read.drop(format!("blist {path} font=\"{font}\""), FONT_DROPPED);
        }
        let Some(value) = el.attr(attr) else { continue };
        let item = format!("blist {path} {attr}=\"{value}\"");
        match (target, map::rgb(value)) {
            (Target::Dropped(why), _) => read.drop(item, format!("dropped: {why}")),
            (Target::Mapped(_), None) => read.drop(item, "dropped: not #rgb or #rrggbb"),
            (Target::Mapped(token), Some(rgb)) => {
                read.tokens.insert(token.to_owned(), walk::color(rgb));
                read.colours.push(ColourRow {
                    source: format!("blist {path} {attr}"),
                    value: rgb,
                    token,
                });
            }
        }
    }
}

/// The file a `file=` names under `dir`, or why it is dropped.
fn file_of<'t>(tree: &'t Tree, dir: &str, file: &str) -> Result<&'t [u8], &'static str> {
    if !map::plain_path(file) {
        return Err("dropped: not a plain relative path");
    }
    tree.get(&format!("{dir}{file}"))
        .map(Vec::as_slice)
        .ok_or("dropped: not in the theme")
}

fn extension(file: &str) -> &str {
    file.rsplit_once('.').map_or("", |(_, e)| e)
}

/// One `<icon id file>` or `<event name file>`.
struct Entry<'a> {
    kind: &'static str,
    key: &'a str,
    file: &'a str,
}

/// Each entry to its slot or cue through its validator, or dropped.
fn read_entries(read: &mut Read, tree: &Tree, dir: &str, entries: &[Entry<'_>], sound: bool) {
    for e in entries {
        let item = format!("{} {} file=\"{}\"", e.kind, e.key, e.file);
        let table: &[(&str, Target)] = if sound { &map::SOUNDS } else { &map::ICONS };
        let target = match map::lookup(table, e.key) {
            Target::Dropped(why) => {
                read.drop(item, format!("dropped: {why}"));
                continue;
            }
            Target::Mapped(t) => t,
        };
        let taken = if sound { &read.sounds } else { &read.icons };
        if taken.contains_key(target) {
            read.drop(item, format!("dropped: {target} is already set"));
            continue;
        }
        let bytes = match file_of(tree, dir, e.file) {
            Ok(b) => b,
            Err(why) => {
                read.drop(item, why);
                continue;
            }
        };
        let (path, checked) = if sound {
            let src = format!("{}{target}.{}", sounds::SOUND_DIR, extension(e.file));
            (
                format!("{}{target}.wav", sounds::SOUND_DIR),
                sounds::check(&src, bytes).map_err(|r| SkinError::new(src.as_str(), r)),
            )
        } else {
            let path = format!("icons/{target}.{}", extension(e.file).to_ascii_lowercase());
            let checked = images::check(&path, bytes).map_err(|r| SkinError::new(path.as_str(), r));
            (path, checked)
        };
        match checked {
            Ok(canonical) => {
                let out = if sound {
                    &mut read.sounds
                } else {
                    &mut read.icons
                };
                out.insert(target.to_owned(), (path, canonical, item));
            }
            Err(err) => read.drop(item, format!("dropped: refused by the validator: {err}")),
        }
    }
}

/// Every `<icon id file>` or `<event name file>` child, in order.
fn entries<'a>(node: &'a Node, element: &'a str, key: &'a str) -> Vec<Entry<'a>> {
    node.children_named(element)
        .map(|c| Entry {
            kind: if element == "icon" { "icon" } else { "sound" },
            key: c.attr(key).unwrap_or_default(),
            file: c.attr("file").unwrap_or_default(),
        })
        .collect()
}

/// Reads a Pidgin theme tree.
///
/// # Errors
/// Past the legacy limits, no `theme.xml`, a refused `theme.xml` (an
/// entity, malformed XML, a root without the `name`/`type` its loader
/// needs).
pub fn read(tree: &Tree) -> Result<Read, ImportError> {
    check_limits(tree)?;
    let root = theme_root(tree)?;
    let mut read = Read::default();
    if let Some((dir, node)) = theme(tree, &root, BLIST)? {
        read.themes
            .push((BLIST, named(&dir, &node, Some(BLIST_TYPE))?));
        read_blist(&mut read, &node);
    }
    if let Some((dir, node)) = theme(tree, &root, ICONS)? {
        read.themes.push((ICONS, named(&dir, &node, None)?));
        read_entries(&mut read, tree, &dir, &entries(&node, "icon", "id"), false);
    }
    if let Some((dir, node)) = theme(tree, &root, SOUND)? {
        read.themes.push((SOUND, named(&dir, &node, Some(SOUND))?));
        read_entries(
            &mut read,
            tree,
            &dir,
            &entries(&node, "event", "name"),
            true,
        );
    }
    Ok(read)
}

fn manifest(id: &str, read: &Read, source: &str, base: SkinBase) -> SkinManifest {
    let paths = |m: &BTreeMap<String, (String, Vec<u8>, String)>| {
        m.iter()
            .map(|(k, (p, ..))| (k.clone(), p.clone()))
            .collect()
    };
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name: id.to_owned(),
        description: "Imported from a Pidgin theme; see the NOTICE beside the package.".to_owned(),
        license: SkinLicense::Text(
            "Third-party Pidgin theme. Not covered by Rekindle's MIT licence; provenance and \
             every change are in the NOTICE beside the package."
                .to_owned(),
        ),
        source: Some(SkinSource {
            format: PIDGIN_FORMAT.to_owned(),
            hash: source.to_owned(),
        }),
        author: None,
        signature: None,
        variants: vec![SkinVariant {
            base,
            contrast: SkinContrast::Standard,
            tokens: TOKENS.to_owned(),
        }],
        regions: BTreeMap::new(),
        icons: paths(&read.icons),
        sounds: paths(&read.sounds),
    }
}

/// Imports the theme tree as the package `id`: a passing package, or a
/// draft naming every failing pair.
///
/// # Errors
/// What [`read`] refuses.
pub fn import(tree: &Tree, id: &str) -> Result<Outcome, ImportError> {
    let read = read(tree)?;
    let source = tree_hash(tree);
    let (skin_base, base) = base_of(&read.tokens);
    let manifest_json =
        serde_json::to_vec(&manifest(id, &read, &source, skin_base)).map_err(|e| {
            ImportError::Xml {
                file: MANIFEST.to_owned(),
                detail: e.to_string(),
            }
        })?;
    let mut files: BTreeMap<String, Vec<u8>> = read
        .icons
        .values()
        .chain(read.sounds.values())
        .map(|(p, b, _)| (p.clone(), b.clone()))
        .collect();
    files.insert(MANIFEST.to_owned(), manifest_json);
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
