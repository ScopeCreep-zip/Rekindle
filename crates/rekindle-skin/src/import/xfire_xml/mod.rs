//! The Xfire XML-era importer (plan E7.7.3 item 1), validated on Carbon
//! Fiber (`.claude/plans/skins/raw/a/xfire-carbon-fiber-unzipped/`;
//! format: `skins/research/a-legacy-formats.md` §1.2).
//!
//! * `Skin.xml` names the files; `Themes.xml`'s first `<Theme>` holds the
//!   `<Colors>` and `<Bitmaps>`; `Info.xml` the name, author and version.
//! * Colours by [`colours::NAMES`]: absolute on a registry token, a signed
//!   shift on the images whose `<Images Color=…>` names them, otherwise
//!   recorded as not mapped or dropped. A non-zero `RGBA()` alpha refuses.
//! * The window frame: the first complete nine-piece set (`TopLeft` …
//!   `BottomRight`, `State="Normal"`, document order) of the chat window's
//!   components ([`FRAME_SOURCE`]) becomes the `window.frame` region,
//!   `nine-slice`, each piece shifted by its `Images` colour and keyed
//!   (`#FF00FF` → alpha). Xfire draws one frame for every window; the
//!   chat window's is the one whose pieces are typed `TopLeft` … in order.
//! * Triggers, layout tiles, other bitmaps, sounds, strings and HTML
//!   templates are dropped and counted in the `NOTICE`.
//! * The package then goes through [`crate::validate::run`]; a failing
//!   pair is settled by [`super::walk::settle`], each change recorded.

pub mod bitmap;
pub mod colours;
mod notice;

use std::collections::BTreeMap;

use rekindle_image::{encode_png, DynamicImage, RgbaImage};
use rekindle_types::dto::skin::{
    RegionFit, RegionSpec, SkinBase, SkinContrast, SkinLicense, SkinManifest, SkinSource,
    SkinVariant, TokenValue, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

use super::tree::{self, Node};
use super::walk::{self, settle};
use super::{tree_hash, ImportError, Imported, Tree};
use crate::colour;
use crate::output::{token_file, zip};
use crate::package::MANIFEST;
use crate::registry::Base;
use colours::{absolute, parse_rgba, target, Target};

/// The `source.format` of an imported package.
pub const XFIRE_XML_FORMAT: &str = "xfire-xml";
/// Where the window frame is read.
pub const FRAME_SOURCE: &str = "Components/ChatWindow.xml";
const FRAME_IMAGE: &str = "assets/window-frame.png";
const TOKENS: &str = "tokens.json";

/// One colour of the theme and what it became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColourRow {
    pub name: String,
    pub value: [i16; 3],
    pub target: Target,
}

/// What the importer read, before the package is built.
#[derive(Debug)]
pub struct Read {
    pub info: BTreeMap<String, String>,
    pub colours: Vec<ColourRow>,
    pub tokens: BTreeMap<String, TokenValue>,
    pub frame: Option<(RgbaImage, rekindle_types::dto::skin::Insets)>,
    pub dropped: BTreeMap<&'static str, usize>,
}

fn file<'t>(tree: &'t Tree, name: &str) -> Result<&'t [u8], ImportError> {
    tree.get(name)
        .map(Vec::as_slice)
        .ok_or_else(|| ImportError::MissingFile {
            file: name.to_owned(),
        })
}

fn xml(tree: &Tree, name: &str) -> Result<Node, ImportError> {
    tree::parse(name, file(tree, name)?)
}

fn descendants<'a>(n: &'a Node, name: &str) -> Vec<&'a Node> {
    let mut out = Vec::new();
    n.descendants(name, &mut out);
    out
}

/// The included files of `Skin.xml`, in order.
fn includes(skin: &Node) -> Vec<String> {
    descendants(skin, "Include")
        .into_iter()
        .filter_map(|n| n.attr("File").map(str::to_owned))
        .collect()
}

fn read_colours(
    theme: &Node,
) -> Result<(Vec<ColourRow>, BTreeMap<String, TokenValue>), ImportError> {
    let mut rows = Vec::new();
    let mut tokens = BTreeMap::new();
    for c in descendants(theme, "Color") {
        let name = c.attr("Name").unwrap_or_default().to_owned();
        let value = parse_rgba(&name, &c.text)?;
        let t = target(&name);
        if let Target::Token(id) = t {
            tokens.insert(id.to_owned(), walk::color(absolute(&name, &c.text, value)?));
        }
        rows.push(ColourRow {
            name,
            value,
            target: t,
        });
    }
    Ok((rows, tokens))
}

/// The first complete nine-piece `Normal` set: (bitmap name, colour name)
/// per piece in [`bitmap::NINE`] order.
fn nine_piece(components: &Node) -> Option<[(String, Option<String>); 9]> {
    let mut found: [Option<(String, Option<String>)>; 9] = Default::default();
    for images in descendants(components, "Images") {
        if images.attr("State").is_some_and(|s| s != "Normal") {
            continue;
        }
        for img in images.children_named("Image") {
            let Some(i) = img
                .attr("Type")
                .and_then(|t| bitmap::NINE.iter().position(|n| *n == t))
            else {
                continue;
            };
            if found[i].is_none() {
                found[i] = Some((img.text.clone(), images.attr("Color").map(str::to_owned)));
            }
        }
        if found.iter().all(Option::is_some) {
            return Some(found.map(Option::unwrap_or_default));
        }
    }
    None
}

fn read_frame(
    tree: &Tree,
    theme: &Node,
    colours: &[ColourRow],
) -> Result<Option<(RgbaImage, rekindle_types::dto::skin::Insets)>, ImportError> {
    if !tree.contains_key(FRAME_SOURCE) {
        return Ok(None);
    }
    let Some(set) = nine_piece(&xml(tree, FRAME_SOURCE)?) else {
        return Ok(None);
    };
    let bitmaps: BTreeMap<&str, &str> = descendants(theme, "Bitmap")
        .into_iter()
        .filter_map(|b| b.attr("Name").map(|n| (n, b.text.as_str())))
        .collect();
    let mut pieces: Vec<RgbaImage> = Vec::with_capacity(9);
    for (name, colour) in &set {
        let path = bitmaps
            .get(name.as_str())
            .ok_or_else(|| ImportError::MissingFile {
                file: format!("Bitmap {name}"),
            })?;
        let mut img = bitmap::decode(path, file(tree, path)?)?;
        // An undefined shift name is the identity (d1 §3.1).
        let by = colour
            .as_deref()
            .and_then(|c| colours.iter().find(|r| r.name == c))
            .map_or([0; 3], |r| r.value);
        bitmap::shift(&mut img, by);
        pieces.push(img);
    }
    let pieces: [RgbaImage; 9] = pieces.try_into().map_err(|_| ImportError::MissingFile {
        file: FRAME_SOURCE.to_owned(),
    })?;
    bitmap::compose(FRAME_SOURCE, &pieces).map(Some)
}

fn count_dropped(tree: &Tree, components: &[Node]) -> BTreeMap<&'static str, usize> {
    let mut d = BTreeMap::new();
    let count = |name: &str| components.iter().map(|c| descendants(c, name).len()).sum();
    d.insert("triggers", count("Trigger"));
    d.insert("layout components", count("Component"));
    d.insert("layout tiles", count("Tile"));
    let html = tree
        .keys()
        .filter(|k| {
            std::path::Path::new(k.as_str())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
        })
        .count();
    d.insert("HTML templates", html);
    d
}

/// Reads `tree` (an unpacked Xfire XML-era skin).
///
/// # Errors
/// A missing or malformed file, a non-zero alpha, a bad colour or image.
pub fn read(tree: &Tree) -> Result<Read, ImportError> {
    let skin = xml(tree, "Skin.xml")?;
    let included = includes(&skin);
    let themes_file = included
        .iter()
        .find(|f| f.ends_with("Themes.xml"))
        .cloned()
        .unwrap_or_else(|| "Themes.xml".to_owned());
    let themes = xml(tree, &themes_file)?;
    let theme = descendants(&themes, "Theme")
        .into_iter()
        .next()
        .ok_or_else(|| ImportError::Xml {
            file: themes_file.clone(),
            detail: "no <Theme>".to_owned(),
        })?;
    let (colours, tokens) = read_colours(theme)?;
    let frame = read_frame(tree, theme, &colours)?;
    let mut components = Vec::new();
    for f in included.iter().filter(|f| f.starts_with("Components/")) {
        if tree.contains_key(f) {
            components.push(xml(tree, f)?);
        }
    }
    let info = match tree.get("Info.xml") {
        Some(bytes) => tree::parse("Info.xml", bytes)?
            .children
            .into_iter()
            .filter(|c| c.children.is_empty() && !c.text.is_empty())
            .fold(BTreeMap::new(), |mut m, c| {
                m.entry(c.name).or_insert(c.text);
                m
            }),
        None => BTreeMap::new(),
    };
    Ok(Read {
        info,
        colours,
        tokens,
        frame,
        dropped: count_dropped(tree, &components),
    })
}

fn base_of(tokens: &BTreeMap<String, TokenValue>) -> (SkinBase, Base) {
    let ground = tokens.get("bg.primary").and_then(|v| match v {
        TokenValue::Color { r, g, b, .. } => Some([*r, *g, *b]),
        _ => None,
    });
    if ground.is_some_and(|g| colour::luminance(g) > 0.179) {
        (SkinBase::Light, Base::Light)
    } else {
        (SkinBase::Dark, Base::Dark)
    }
}

fn manifest(id: &str, read: &Read, source: &str, base: SkinBase) -> SkinManifest {
    let name: String = read
        .info
        .get("Name")
        .map_or_else(|| id.to_owned(), |n| n.chars().take(64).collect());
    let mut regions = BTreeMap::new();
    if let Some((_, insets)) = &read.frame {
        regions.insert(
            "window.frame".to_owned(),
            RegionSpec {
                image: FRAME_IMAGE.to_owned(),
                fit: RegionFit::NineSlice(*insets),
                states: BTreeMap::new(),
                tint: None,
                mask: None,
                text_backing: None,
            },
        );
    }
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name,
        description: "Imported from an Xfire XML-era skin; see the NOTICE beside the package."
            .to_owned(),
        license: SkinLicense::Text(
            "Third-party Xfire skin art, published without a licence. Not covered by Rekindle's \
             MIT licence; provenance and every change are in the NOTICE beside the package."
                .to_owned(),
        ),
        source: Some(SkinSource {
            format: XFIRE_XML_FORMAT.to_owned(),
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

/// Imports `tree` as the package `id`: read, build, validate, and settle
/// every failing pair by the walk.
///
/// # Errors
/// What [`read`] refuses, an image that does not encode, a refusal the
/// walk cannot settle.
pub fn import(tree: &Tree, id: &str) -> Result<Imported, ImportError> {
    let read = read(tree)?;
    let source = tree_hash(tree);
    let (skin_base, base) = base_of(&read.tokens);
    let manifest = manifest(id, &read, &source, skin_base);
    let manifest_json = serde_json::to_vec(&manifest).map_err(|e| ImportError::Xml {
        file: MANIFEST.to_owned(),
        detail: e.to_string(),
    })?;
    let frame_png = match &read.frame {
        Some((img, _)) => Some(encode_png(&DynamicImage::ImageRgba8(img.clone())).map_err(
            |e| ImportError::Image {
                file: FRAME_IMAGE.to_owned(),
                detail: e.to_string(),
            },
        )?),
        None => None,
    };
    let build = |tokens: &BTreeMap<String, TokenValue>| {
        let mut files = BTreeMap::new();
        files.insert(MANIFEST.to_owned(), manifest_json.clone());
        files.insert(
            TOKENS.to_owned(),
            token_file(TOKENS, tokens).map_err(ImportError::Refused)?,
        );
        if let Some(png) = &frame_png {
            files.insert(FRAME_IMAGE.to_owned(), png.clone());
        }
        zip(&files).map_err(ImportError::Refused)
    };
    let mut tokens = read.tokens.clone();
    let (settled, changes) = settle(&mut tokens, base, build)?;
    let canonical = match settled {
        walk::WalkResult::Passed(c) => c,
        walk::WalkResult::Unsettled(u) => {
            return Err(ImportError::Unsettled {
                error: u.error,
                why: u.why,
            })
        }
    };
    let notice = notice::render(id, &read, &source, &changes);
    Ok(Imported {
        canonical,
        notice,
        changes,
    })
}

#[cfg(test)]
mod tests;
