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
//! * Classic v3 ([`v3`]): lower-case files and attributes in either case,
//!   attribute-form colours, every component in `xfirecomponents.xml`.
//!   Files and attributes are matched ASCII-case-insensitively throughout.
//! * Licence ([`super::credit`]): a tree whose `Info.xml` names
//!   [`XFIRE_AUTHOR`] as its original author, or that an [`XfireCredit`]
//!   proves Xfire's, is Xfire's own art, usable under the owner's
//!   2026-10-15 ruling (Xfire is defunct); any other author's is
//!   third-party work published without a licence. Both say Rekindle's MIT
//!   licence does not cover the pack.
//! * Triggers, layout tiles, other bitmaps, sounds, strings and HTML
//!   templates are dropped and counted in the `NOTICE`.
//! * The package then goes through [`crate::validate::run`]; a failing
//!   pair is settled by [`super::walk::settle`], each change recorded.

pub mod bitmap;
pub mod colours;
pub mod controls;
mod decor;
mod notice;
pub mod refs;
mod row_decor;
mod sources;
mod stack;
pub mod surfaces;
pub mod tiles;
use sources::{component_files, count_dropped};
pub(crate) mod title;
mod title_rows;
pub mod v3;

use std::collections::BTreeMap;

use rekindle_image::{encode_png, DynamicImage, RgbaImage};
use rekindle_types::dto::skin::{
    RegionFit, RegionSpec, SkinBase, SkinContrast, SkinLicense, SkinManifest, SkinSource,
    SkinVariant, SkinWindow, TokenValue, WindowLayout, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

use super::credit::{Authorship, XfireCredit};
use super::credits;
use super::tree::{self, Node};
use super::walk::{self, settle};
use super::xfire_ini::lookup;
use super::{tree_hash, Control, ImportError, Imported, Tree};
use crate::colour;
use crate::output::{token_file, zip};
use crate::package::MANIFEST;
use crate::registry::Base;
use crate::shape;
use colours::{absolute, parse_attrs, parse_rgba, target_in, Target};

/// The `source.format` of an imported package.
pub const XFIRE_XML_FORMAT: &str = "xfire-xml";
/// Where the window frame is read.
pub const FRAME_SOURCE: &str = "Components/ChatWindow.xml";
const FRAME_IMAGE: &str = "assets/window-frame.png";
const TOKENS: &str = "tokens.json";
/// The original author `Info.xml` names on Xfire's own skins (Symbiosis).
pub use super::credits::XFIRE_AUTHOR;

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
    /// `Info.xml`'s first `<Author Type="Original">`.
    pub original_author: Option<String>,
    /// Every `<Author Type="Original">`, then every `Type="Contributor"`,
    /// in file order (plan E7.7.3d).
    pub authors: Vec<String>,
    pub contributors: Vec<String>,
    pub colours: Vec<ColourRow>,
    pub tokens: BTreeMap<String, TokenValue>,
    pub frame: Option<(RgbaImage, rekindle_types::dto::skin::Insets)>,
    /// Where and by which rule the frame was read, for the `NOTICE`.
    pub frame_source: String,
    pub dropped: BTreeMap<&'static str, usize>,
    /// The painted control slots ([`controls`]) and what they kept.
    pub controls: Vec<Control>,
    pub tally: controls::StateTally,
    /// Which layout the tree is.
    pub layout: Layout,
    /// The colour roles the components name ([`refs`]).
    pub refs: Vec<refs::RefsApplied>,
    /// Each window's frame layout and its tile images (plan SK0 S1).
    pub windows: Windows,
}

/// The frame layouts of a package and the tile images they name.
pub type Windows = BTreeMap<SkinWindow, (WindowLayout, BTreeMap<String, Vec<u8>>)>;

/// The two XML-era layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Symbiosis and Carbon Fiber: `Components/*.xml`, `RGBA()` colours.
    Late,
    /// Classic v3: [`v3::SOURCE`], attribute colours, v3's names.
    ClassicV3,
}

impl Layout {
    fn names(self) -> &'static [(&'static str, Target)] {
        match self {
            Self::Late => colours::NAMES,
            Self::ClassicV3 => v3::NAMES,
        }
    }
}

/// A file of the tree, matched exactly, else ASCII-case-insensitively (a
/// relative path only, as [`lookup`] checks).
fn file<'t>(tree: &'t Tree, name: &str) -> Result<&'t [u8], ImportError> {
    lookup(tree, name).map(|(_, bytes)| bytes)
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
        .filter_map(|n| n.attr_ci("File").map(str::to_owned))
        .collect()
}

fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// The theme's bitmaps: name to file.
fn bitmaps(theme: &Node) -> BTreeMap<&str, &str> {
    descendants(theme, "Bitmap")
        .into_iter()
        .filter_map(|b| b.attr_ci("Name").map(|n| (n, b.text.as_str())))
        .collect()
}

/// The bitmap `name`, decoded and shifted by the colour `shift` (an
/// undefined name is the identity, d1 §3.1). `name` is a theme
/// `<Bitmap Name=…>`, or else a file of the tree (Onyx, Space Cowboy and
/// Total Chaos name their tiles' images by path, `chatskin/chat-topleft.gif`),
/// read through [`lookup`]'s relative-path check like every other file.
fn piece(
    tree: &Tree,
    bitmaps: &BTreeMap<&str, &str>,
    colours: &[ColourRow],
    name: &str,
    shift: Option<&str>,
) -> Result<RgbaImage, ImportError> {
    let path = match bitmaps.get(name) {
        Some(path) => *path,
        None if lookup(tree, name).is_ok() => name,
        None => {
            return Err(ImportError::UndefinedBitmap {
                name: name.to_owned(),
            })
        }
    };
    let mut img = bitmap::decode(path, file(tree, path)?)?;
    let by = shift
        .and_then(|c| colours.iter().find(|r| r.name == c))
        .map_or([0; 3], |r| r.value);
    bitmap::shift(&mut img, by);
    Ok(img)
}

fn read_colours(
    theme: &Node,
    layout: Layout,
) -> Result<(Vec<ColourRow>, BTreeMap<String, TokenValue>), ImportError> {
    let mut rows = Vec::new();
    let mut tokens = BTreeMap::new();
    for c in descendants(theme, "Color") {
        let name = c.attr_ci("Name").unwrap_or_default().to_owned();
        let value = parse_attrs(&name, |k| c.attr_ci(k).map(str::to_owned))
            .unwrap_or_else(|| parse_rgba(&name, &c.text))?;
        let t = target_in(layout.names(), &name);
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
    nine_piece_in(descendants(components, "Images"))
}

/// [`nine_piece`] over the image sets `sets`, in order.
fn nine_piece_in<'a>(
    sets: impl IntoIterator<Item = &'a Node>,
) -> Option<[(String, Option<String>); 9]> {
    let mut found: [Option<(String, Option<String>)>; 9] = Default::default();
    for images in sets {
        if images
            .attr_ci("State")
            .is_some_and(|s| !s.eq_ignore_ascii_case("Normal"))
        {
            continue;
        }
        for img in images.children_named("Image") {
            let Some(i) = img
                .attr_ci("Type")
                .and_then(|t| bitmap::NINE.iter().position(|n| n.eq_ignore_ascii_case(t)))
            else {
                continue;
            };
            if found[i].is_none() {
                found[i] = Some((img.text.clone(), images.attr_ci("Color").map(str::to_owned)));
            }
        }
        if found.iter().all(Option::is_some) {
            return Some(found.map(Option::unwrap_or_default));
        }
    }
    None
}

/// The frame and where it was read.
type ReadFrame = (
    Option<(RgbaImage, rekindle_types::dto::skin::Insets)>,
    String,
    Option<[RgbaImage; 9]>,
);

/// The file `Skin.xml`'s `section` (`<ChatSkin>`, `<MainSkin>`) includes:
/// that window's layout.
fn window_layout<'s>(skin: &'s Node, section: &str) -> Option<&'s str> {
    skin.children
        .iter()
        .find(|n| n.name.eq_ignore_ascii_case(section))
        .and_then(|n| descendants(n, "Include").into_iter().next())
        .and_then(|i| i.attr_ci("File"))
}

fn read_frame(
    tree: &Tree,
    (skin, theme): (&Node, &Node),
    colours: &[ColourRow],
    layout: Layout,
    components: &[Node],
) -> Result<ReadFrame, ImportError> {
    let bitmaps = bitmaps(theme);
    let decode = |name: &str, shift: Option<&str>| piece(tree, &bitmaps, colours, name, shift);
    if layout == Layout::ClassicV3 {
        let source = "xfirecomponents.xml (the chat window's outer edges' middles, \
                      9 px rails; no top or bottom row; transparent middle)";
        let read = v3::frame(&xml(tree, v3::SOURCE)?, decode)?;
        let (frame, pieces) = read.map(|(i, n, p)| ((i, n), p)).unzip();
        return Ok((frame, source.to_owned(), pieces));
    }
    let set = if lookup(tree, FRAME_SOURCE).is_ok() {
        nine_piece(&xml(tree, FRAME_SOURCE)?)
    } else {
        None
    };
    let Some(set) = set else {
        let Some(file) = window_layout(skin, "ChatSkin").filter(|f| lookup(tree, f).is_ok()) else {
            return Ok((None, String::new(), None));
        };
        return Ok(match tiles::frame(&xml(tree, file)?, components, decode)? {
            Some((img, insets, shape, pieces)) => (
                Some((img, insets)),
                format!("the chat window layout {file}: {}", shape.describe()),
                Some(pieces),
            ),
            None => (None, String::new(), None),
        });
    };
    let mut pieces: Vec<RgbaImage> = Vec::with_capacity(9);
    for (name, colour) in &set {
        pieces.push(decode(name, colour.as_deref())?);
    }
    let pieces: [RgbaImage; 9] = pieces.try_into().map_err(|_| ImportError::MissingFile {
        file: FRAME_SOURCE.to_owned(),
    })?;
    let frame = bitmap::compose(FRAME_SOURCE, &pieces)?;
    Ok((Some(frame), FRAME_SOURCE.to_owned(), Some(pieces)))
}

/// Reads `tree` (an unpacked Xfire XML-era skin).
///
/// # Errors
/// A missing or malformed file, a non-zero alpha, a bad colour or image.
pub fn read(tree: &Tree) -> Result<Read, ImportError> {
    let skin = xml(tree, "Skin.xml")?;
    let included = includes(&skin);
    let layout = if included.iter().any(|f| f.eq_ignore_ascii_case(v3::SOURCE)) {
        Layout::ClassicV3
    } else {
        Layout::Late
    };
    let themes_file = included
        .iter()
        .find(|f| ends_with_ci(f, "Themes.xml"))
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
    let (colours, mut tokens) = read_colours(theme, layout)?;
    let components = component_files(tree, &included)?;
    let refs = refs::apply(&refs::xml_refs(&components), &colours, &mut tokens);
    let (frame, frame_source, chat) =
        read_frame(tree, (&skin, theme), &colours, layout, &components)?;
    let (mut controls, tally) = controls::read(tree, theme, &colours, &components)?;
    let mut windows = Windows::new();
    if let Some(f) = window_layout(&skin, "MainSkin").filter(|f| lookup(tree, f).is_ok()) {
        let main = xml(tree, f)?;
        title_rows::crop(&main, &mut controls, &tally.sources);
        let bitmaps = bitmaps(theme);
        let decode = |name: &str, shift: Option<&str>| piece(tree, &bitmaps, &colours, name, shift);
        decor::main_window(&main, &components, decode, &colours, &mut windows)?;
    }
    if let Some(pieces) = &chat {
        windows.insert(
            SkinWindow::Chat,
            shape::from_nine(SkinWindow::Chat, pieces)?,
        );
    }
    let info_xml = match lookup(tree, "Info.xml") {
        Ok((key, bytes)) => Some(tree::parse(key, bytes)?),
        Err(_) => None,
    };
    let authors_of = |kind: &str| -> Vec<String> {
        info_xml.as_ref().map_or_else(Vec::new, |n| {
            n.children_named("Author")
                .filter(|a| {
                    a.attr_ci("Type")
                        .is_some_and(|t| t.eq_ignore_ascii_case(kind))
                })
                .map(|a| a.text.clone())
                .collect()
        })
    };
    let (authors, contributors) = (authors_of("Original"), authors_of("Contributor"));
    let original_author = authors.first().cloned();
    let info = info_xml.map_or_else(BTreeMap::new, |n| {
        n.children
            .into_iter()
            .filter(|c| c.children.is_empty() && !c.text.is_empty())
            .fold(BTreeMap::new(), |mut m, c| {
                m.entry(c.name).or_insert(c.text);
                m
            })
    });
    Ok(Read {
        info,
        original_author,
        authors,
        contributors,
        colours,
        tokens,
        frame,
        frame_source,
        dropped: count_dropped(tree, &components),
        controls,
        tally,
        layout,
        refs,
        windows,
    })
}

impl Read {
    /// Whether `Info.xml` names Xfire, Inc. as the original author.
    #[must_use]
    pub fn is_xfire_own(&self) -> bool {
        self.original_author.as_deref() == Some(XFIRE_AUTHOR)
    }
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

impl Read {
    /// Whose art the pack is: `Info.xml`'s original author first, else the
    /// credit given, else third-party.
    #[must_use]
    pub fn authorship<'c>(&self, credit: Option<&'c XfireCredit>) -> Authorship<'c> {
        match credit {
            _ if self.is_xfire_own() => Authorship::XfireByInfo,
            Some(c) => Authorship::XfireByCredit(c),
            None => Authorship::ThirdParty,
        }
    }
}

fn manifest(
    id: &str,
    read: &Read,
    (source, tree): (&str, &Tree),
    base: SkinBase,
    (authorship, credit): (Authorship<'_>, Option<&XfireCredit>),
) -> SkinManifest {
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
    for c in &read.controls {
        regions.insert(c.region.to_owned(), c.spec());
    }
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name,
        description: "Imported from an Xfire XML-era skin; see the NOTICE beside the package."
            .to_owned(),
        license: SkinLicense::Text(authorship.manifest_text()),
        source: Some(SkinSource {
            format: XFIRE_XML_FORMAT.to_owned(),
            hash: source.to_owned(),
        }),
        credits: Some(credits::build(
            tree,
            &read.authors,
            &read.contributors,
            read.info.get("Website").map(String::as_str),
            credit,
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
        windows: read
            .windows
            .iter()
            .map(|(w, (layout, _))| (*w, layout.clone()))
            .collect(),
    }
}

/// Imports `tree` as the package `id`: read, build, validate, and settle
/// every failing pair by the walk.
///
/// # Errors
/// What [`read`] refuses, an image that does not encode, a refusal the
/// walk cannot settle.
pub fn import(
    tree: &Tree,
    id: &str,
    credit: Option<&XfireCredit>,
) -> Result<Imported, ImportError> {
    let read = read(tree)?;
    let source = tree_hash(tree);
    let (skin_base, base) = base_of(&read.tokens);
    let authorship = read.authorship(credit);
    let manifest = manifest(id, &read, (&source, tree), skin_base, (authorship, credit));
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
    let mut control_files = BTreeMap::new();
    for c in &read.controls {
        control_files.extend(c.files()?);
    }
    for (_, files) in read.windows.values() {
        control_files.extend(files.clone());
    }
    let build = |tokens: &BTreeMap<String, TokenValue>| {
        let mut files = control_files.clone();
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
    let notice = notice::render(id, &read, &source, authorship, &changes);
    Ok(Imported {
        canonical,
        notice,
        changes,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod fan_tests;

#[cfg(test)]
mod surfaces_tests;

#[cfg(test)]
mod chat_surfaces_tests;

#[cfg(test)]
mod dialog_surfaces_tests;
