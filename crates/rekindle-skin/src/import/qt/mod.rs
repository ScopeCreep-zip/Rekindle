//! The Qt-palette importers (plan E7.7.3 item 5): Mumble themes and
//! TeamSpeak 3 styles. Both are a QSS stylesheet carrying a `QPalette`
//! plus image files, so they share one reader with two front-ends:
//! * [`mumble`]: `theme.ini` → the first style's QSS →
//!   its `ApplicationPalette` block; PNG and SVG icons anywhere in the
//!   theme;
//! * [`teamspeak`]: `styles/<name>.qss` → the header's `QPalette::`
//!   overrides; `styles/<name>/media/*.png`.
//!
//! [`palette`] maps the roles to registry tokens; [`background`] reads the
//! stylesheet's whole-widget `background-image` files inside the theme
//! onto region slots (plan E7.7.7 S4 part 3); [`svg`] rasterises SVG
//! with every reference refused. An image fills the status slot its file
//! stem names (`online.png` → `status.online`, the registry's
//! [`IconSlot`] ids without `status.`), through [`crate::images::check`].
//! Every item that does not enter the package is named in the NOTICE; the
//! rest settles to [`Outcome::Passed`] or an [`Outcome::Draft`] through
//! [`conclude`] like every importer.

pub mod background;
pub mod mumble;
mod notice;
pub mod palette;
pub mod svg;
pub mod teamspeak;

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{
    SkinBase, SkinContrast, SkinError, SkinLicense, SkinManifest, SkinSource, SkinVariant,
    TokenValue, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

use super::pidgin::{ColourRow, DroppedItem};
use super::{base_of, conclude, credits, tree_hash, walk, ImportError, Outcome, Tree};
use crate::images;
use crate::output::{token_file, zip};
use crate::package::MANIFEST;
use crate::registry::IconSlot;
use palette::Target;

const TOKENS: &str = "tokens.json";

/// Which theme system a tree came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Front {
    Mumble,
    TeamSpeak3,
}

impl Front {
    /// The manifest's `source.format`.
    #[must_use]
    pub const fn format(self) -> &'static str {
        match self {
            Self::Mumble => mumble::MUMBLE_FORMAT,
            Self::TeamSpeak3 => teamspeak::TEAMSPEAK_FORMAT,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Mumble => "Mumble theme",
            Self::TeamSpeak3 => "TeamSpeak 3 style",
        }
    }
}

/// What a front-end read, before the package is built.
#[derive(Debug)]
pub struct Read {
    pub front: Front,
    /// Each file the palette came from, in reading order.
    pub files: Vec<String>,
    pub colours: Vec<ColourRow>,
    pub tokens: BTreeMap<String, TokenValue>,
    /// Status slot → (package path, canonical PNG, source file).
    pub icons: BTreeMap<String, (String, Vec<u8>, String)>,
    /// Region slot → its background images ([`background`]).
    pub regions: BTreeMap<&'static str, background::SlotImages>,
    /// (SVG file, reference) for every image href the resolver refused.
    pub refused: Vec<(String, String)>,
    pub dropped: Vec<DroppedItem>,
    /// The authors the pack states: a TeamSpeak 3 `package.ini` `Author`
    /// (Mumble's `theme.ini` names none).
    pub authors: Vec<String>,
}

impl Read {
    fn new(front: Front) -> Self {
        Self {
            front,
            files: Vec::new(),
            colours: Vec::new(),
            tokens: BTreeMap::new(),
            icons: BTreeMap::new(),
            regions: BTreeMap::new(),
            refused: Vec::new(),
            dropped: Vec::new(),
            authors: Vec::new(),
        }
    }

    fn drop(&mut self, item: impl Into<String>, why: impl Into<String>) {
        self.dropped.push(DroppedItem {
            item: item.into(),
            why: why.into(),
        });
    }

    /// One palette entry: `role` set to `value`, read from `source`.
    fn colour(&mut self, source: &str, role: &str, value: &str) {
        let item = format!("{source} {role} = {value}");
        match (palette::role(role), palette::rgb(value)) {
            (Target::Dropped(why), _) => self.drop(item, format!("dropped: {why}")),
            (Target::Mapped(_), None) => self.drop(item, "dropped: not #rgb or #rrggbb"),
            (Target::Mapped(token), Some(_)) if self.tokens.contains_key(token) => {
                self.drop(item, format!("dropped: {token} is already set"));
            }
            (Target::Mapped(token), Some(rgb)) => {
                self.tokens.insert(token.to_owned(), walk::color(rgb));
                self.colours.push(ColourRow {
                    source: format!("{source} {role}"),
                    value: rgb,
                    token,
                });
            }
        }
    }

    /// One image file: to the slot its stem names, through the validator.
    fn image(&mut self, path: &str, bytes: &[u8]) {
        let name = path.rsplit('/').next().unwrap_or(path);
        let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
        let ext = ext.to_ascii_lowercase();
        let Some(slot) = IconSlot::ALL
            .into_iter()
            .find(|s| s.id().strip_prefix("status.") == Some(stem))
        else {
            return self.drop(path, "dropped: the name is not a status slot");
        };
        if self.icons.contains_key(slot.id()) {
            return self.drop(path, format!("dropped: {} is already set", slot.id()));
        }
        let png = match ext.as_str() {
            "png" => bytes.to_vec(),
            "svg" => match svg::rasterise(bytes) {
                Ok(r) => {
                    let refs = r.refused.into_iter().map(|h| (path.to_owned(), h));
                    self.refused.extend(refs);
                    r.png
                }
                Err(why) => return self.drop(path, format!("dropped: SVG refused: {why}")),
            },
            _ => return self.drop(path, "dropped: not PNG or SVG"),
        };
        let out = format!("icons/{}.png", slot.id());
        match images::check(&out, &png) {
            Ok(canonical) => {
                self.icons
                    .insert(slot.id().to_owned(), (out, canonical, path.to_owned()));
            }
            Err(rule) => {
                let err = SkinError::new(out.as_str(), rule);
                self.drop(path, format!("dropped: refused by the validator: {err}"));
            }
        }
    }
}

/// A folder prefix that is the root (`""`) or one top-level folder.
fn top_level(at: &str) -> bool {
    at.is_empty() || (at.ends_with('/') && at.matches('/').count() == 1)
}

/// The folder holding `file`: the root, or one top-level folder.
fn root_of(tree: &Tree, file: &str) -> Option<String> {
    tree.keys().find_map(|k| {
        let at = k.strip_suffix(file)?;
        top_level(at).then(|| at.to_owned())
    })
}

/// A UTF-8 text file of the tree.
fn text<'t>(tree: &'t Tree, file: &str) -> Result<&'t str, ImportError> {
    let bytes = tree.get(file).ok_or_else(|| ImportError::MissingFile {
        file: file.to_owned(),
    })?;
    std::str::from_utf8(bytes).map_err(|_| ImportError::Format {
        file: file.to_owned(),
        detail: "not UTF-8".to_owned(),
    })
}

/// An INI file as (section, key, value), keys outside a section under "".
fn ini(text: &str) -> Vec<(String, String, String)> {
    let mut section = String::new();
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            s.trim().clone_into(&mut section);
        } else if let Some((k, v)) = line.split_once('=').filter(|_| !line.starts_with(';')) {
            let v = v.trim().trim_matches('"');
            out.push((section.clone(), k.trim().to_owned(), v.to_owned()));
        }
    }
    out
}

fn manifest(tree: &Tree, id: &str, read: &Read, source: &str, base: SkinBase) -> SkinManifest {
    let what = read.front.name();
    SkinManifest {
        text: BTreeMap::new(),
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name: id.to_owned(),
        description: format!("Imported from a {what}; see the NOTICE beside the package."),
        license: SkinLicense::Text(format!(
            "Third-party {what}. Not covered by Rekindle's MIT licence; provenance and every \
             change are in the NOTICE beside the package."
        )),
        source: Some(SkinSource {
            format: read.front.format().to_owned(),
            hash: source.to_owned(),
        }),
        credits: Some(credits::build(tree, &read.authors, &[], None, None)),
        author: None,
        signature: None,
        variants: vec![SkinVariant {
            base,
            contrast: SkinContrast::Standard,
            tokens: TOKENS.to_owned(),
        }],
        regions: read
            .regions
            .iter()
            .map(|(slot, images)| ((*slot).to_owned(), background::spec(slot, images)))
            .collect(),
        icons: read
            .icons
            .iter()
            .map(|(slot, (path, ..))| (slot.clone(), path.clone()))
            .collect(),
        sounds: BTreeMap::new(),
        windows: BTreeMap::new(),
    }
}

/// Builds the package `id` from what a front-end read.
fn package(tree: &Tree, read: &Read, id: &str) -> Result<Outcome, ImportError> {
    let source = tree_hash(tree);
    let (skin_base, base) = base_of(&read.tokens);
    let manifest_json =
        serde_json::to_vec(&manifest(tree, id, read, &source, skin_base)).map_err(|e| {
            ImportError::Format {
                file: MANIFEST.to_owned(),
                detail: e.to_string(),
            }
        })?;
    let mut files: BTreeMap<String, Vec<u8>> = read
        .icons
        .values()
        .chain(read.regions.values().flat_map(BTreeMap::values))
        .map(|(path, bytes, _)| (path.clone(), bytes.clone()))
        .collect();
    files.insert(MANIFEST.to_owned(), manifest_json);
    let build = |tokens: &BTreeMap<String, TokenValue>| {
        let mut files = files.clone();
        let tokens = token_file(TOKENS, tokens).map_err(ImportError::Refused)?;
        files.insert(TOKENS.to_owned(), tokens);
        zip(&files).map_err(ImportError::Refused)
    };
    conclude(read.tokens.clone(), base, build, |changes| {
        notice::render(id, read, &source, changes)
    })
}

#[cfg(test)]
mod tests;
