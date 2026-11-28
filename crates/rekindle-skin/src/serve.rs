//! What the `skin://` scheme serves (plan E7.7.2 "`skin://` scheme"): the
//! files of one canonical package, addressed by content, and the headers
//! they go out with. Pure functions: the Host looks the package up, the
//! shell copies the answer into an HTTP response.
//!
//! A URL is `{origin}/{package}/{file}`: `package` is the canonical
//! package's BLAKE3 address (64 lowercase hex) and `file` is either
//! [`TOKENS_CSS`] (the active variant's `@layer skin` tokens followed by
//! the region rules, [`stylesheet`]) or `{blake3}.{png|gif}`, one
//! canonical image by its own address (the hrefs
//! [`crate::regions::css::skin_css`] writes), the reduced-motion still of
//! an animated region GIF among them ([`crate::regions::still`], derived
//! from the canonical GIF, so its address is stable). Every other path is refused
//! before anything is looked up, so no `..`, no second segment and no
//! query reaches a store.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{
    SkinBase, SkinContrast, SkinError, SkinManifest, SkinRule, SkinVariant, SkinWindow, TokenValue,
};
use rekindle_types::dto::skin_action::{MenuRegistry, PackUrl, SkinAction};
use rekindle_types::dto::skin_library::{SkinPaletteColour, SkinTileMask, SkinWindowFrame};
use rekindle_types::dto::skin_tree::{NodeGlyph, SkinNode, SkinShell};

use crate::generate::tui_colours;
use crate::output::tokens_css;
use crate::package::{self, Package};
use crate::regions::css::skin_css;
use crate::regions::labels::label_css;
use crate::regions::title::title_css;
use crate::registry::{resolve, Base, TokenEntry, ENTRIES};
use crate::shape;
use crate::tokens;

mod image;

/// The stylesheet's file name.
pub const TOKENS_CSS: &str = "tokens.css";

/// The scheme's name, as the shell registers it.
pub const SCHEME: &str = "skin";

/// The scheme's origin: `skin://localhost` on macOS and Linux,
/// `http://skin.localhost` on Windows, where WebView2 serves a custom
/// scheme under that name (`tauri` `protocol/mod.rs`).
#[must_use]
pub const fn origin() -> &'static str {
    if cfg!(windows) {
        "http://skin.localhost"
    } else {
        "skin://localhost"
    }
}

/// The href of a package's stylesheet.
#[must_use]
pub fn tokens_href(package: &str) -> String {
    format!("{}/{package}/{TOKENS_CSS}", origin())
}

/// How long the webview may keep a served file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cache {
    /// A built-in pack: its bytes never change under an address.
    Immutable,
    /// The identity's library and the login copy: never in the webview's
    /// disk cache, so nothing survives a lock there (d2 §5).
    NoStore,
}

impl Cache {
    /// The `Cache-Control` value.
    #[must_use]
    pub const fn header(self) -> &'static str {
        match self {
            Self::Immutable => "public, max-age=31536000, immutable",
            Self::NoStore => "no-store",
        }
    }
}

/// One file the path names inside its package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum File<'a> {
    /// [`TOKENS_CSS`].
    Stylesheet,
    /// A canonical image: its BLAKE3 hex and extension.
    Image { hash: &'a str, ext: &'a str },
}

/// 64 lowercase hex digits: a BLAKE3 address.
#[must_use]
pub fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `/{package}/{file}` (the URL's path), or `None` for any other shape.
#[must_use]
pub fn parse_path(path: &str) -> Option<(&str, File<'_>)> {
    let (package, file) = path.strip_prefix('/')?.split_once('/')?;
    if !is_hash(package) {
        return None;
    }
    if file == TOKENS_CSS {
        return Some((package, File::Stylesheet));
    }
    let (hash, ext) = file.split_once('.')?;
    (is_hash(hash) && matches!(ext, "png" | "gif")).then_some((package, File::Image { hash, ext }))
}

/// A served file's body and media type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

/// The variant a skin is applied with: its first standard-contrast
/// variant (the forced-colours and high-contrast overrides are `@layer
/// a11y`'s), else its first.
pub(crate) fn active(variants: &[SkinVariant]) -> Option<&SkinVariant> {
    variants
        .iter()
        .find(|v| v.contrast == SkinContrast::Standard)
        .or_else(|| variants.first())
}

fn unreadable(detail: &str) -> SkinError {
    SkinError::new(
        SkinError::PACKAGE,
        SkinRule::ManifestSyntax {
            detail: detail.to_owned(),
        },
    )
}

/// What a window applies: the active variant's base, the stylesheet and
/// the region slots the skin paints, and the TUI's palette subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub base: SkinBase,
    pub css: String,
    pub regions: Vec<String>,
    /// [`sheet_images`] of the shared rules: every image href any window's sheet may draw (SK4).
    pub images: Vec<String>,
    /// Art from the frame and title rules scoped to each framed window.
    pub frame_images: BTreeMap<SkinWindow, Vec<String>>,
    /// [`crate::generate::tui_colours`] of the active variant.
    pub palette: Vec<SkinPaletteColour>,
    /// Each tree-laid window kind's shell outline (plan SK2-S2).
    pub shells: Vec<SkinShell>,
    /// [`crate::skin_tree::brand::title_logo`]: the strip logo's brand glyph (design `br`).
    pub title_logo: Option<NodeGlyph>,
}

/// The active variant of the read package `p` and its tokens after the
/// defaults merge.
///
/// # Errors
/// The manifest names no variant, or its token file is missing or does
/// not parse.
pub fn active_resolved<'m>(
    p: &Package<'_>,
    m: &'m SkinManifest,
) -> Result<(&'m SkinVariant, Vec<(&'static TokenEntry, TokenValue)>), SkinError> {
    let v = active(&m.variants).ok_or_else(|| unreadable("no variant"))?;
    let file = p
        .get(&v.tokens)
        .ok_or_else(|| unreadable("no token file"))?;
    let skin = tokens::parse(&v.tokens, file)?;
    Ok((v, resolve(ENTRIES, Base::of(v.base, v.contrast), &skin)))
}

pub(crate) fn files(p: &Package<'_>) -> BTreeMap<String, Vec<u8>> {
    p.files().map(|(n, b)| (n.to_owned(), b.to_vec())).collect()
}

/// The stylesheet of the canonical package `bytes` at `address`: resolved tokens, the region rules
/// (images under `{origin}/{address}`), then the window frames ([`shape::frame_css`], SK0 S2).
///
/// # Errors
/// The package, its manifest or its tokens do not read, or a region does
/// not emit (never for a package the helper returned).
pub fn stylesheet(bytes: &[u8], address: &str) -> Result<Stylesheet, SkinError> {
    let p = package::read(bytes)?;
    let (m, made) = crate::conform::served(bytes, &p, address)?;
    let (v, resolved) = active_resolved(&p, &m)?;
    let mut css = tokens_css(&resolved).ok_or_else(|| unreadable("tokens do not emit"))?;
    let base = format!("{}/{address}", origin());
    let mut images = files(&p);
    images.extend(made);
    css.push_str(&skin_css(&m, &images, &base).ok_or_else(|| unreadable("regions"))?);
    css.push_str(&title_css(&m, &images, &resolved));
    css.push_str(&label_css(&m, &images, &resolved));
    let (frames, trees) =
        shape::frame_parts(&m, &images, &base).ok_or_else(|| unreadable("frame"))?;
    let shared = sheet_images(&css, &base);
    let frame_images = frames
        .iter()
        .map(|frame| {
            let mut rules = frame.tiles.clone();
            rules.push_str(&frame.title);
            let own = sheet_images(&rules, &base)
                .into_iter()
                .filter(|href| !shared.contains(href))
                .collect();
            (frame.window, own)
        })
        .collect();
    css.push_str(&shape::layered(&frames, &trees));
    Ok(Stylesheet {
        base: v.base,
        shells: shells(&m, &trees, &shared, &base)?,
        images: shared,
        frame_images,
        css,
        regions: m.regions.keys().cloned().collect(),
        palette: tui_colours(&resolved),
        title_logo: crate::skin_tree::brand::title_logo(&m),
    })
}

/// Each tree-laid window kind's shell and each of its popups', in
/// [`shape::frame_parts`]' order: its served minimum ([`shape::min_window`])
/// and the images its own rules draw that `shared` does not list (plan
/// SK4: a window decodes only the shared images and its own shell's).
fn shells(
    m: &SkinManifest,
    trees: &[shape::TreeSheet],
    shared: &[String],
    base: &str,
) -> Result<Vec<SkinShell>, SkinError> {
    let own = |css: &str| -> Vec<String> {
        sheet_images(css, base)
            .into_iter()
            .filter(|h| !shared.contains(h))
            .collect()
    };
    let menus = MenuRegistry::for_pack(&m.name, pack_link(m).as_ref());
    trees
        .iter()
        .map(|t| {
            let layout = m.windows.get(&t.window).ok_or_else(|| unreadable("tree"))?;
            let mut shell = match &t.popup {
                None => {
                    let tree = layout.tree.as_ref().ok_or_else(|| unreadable("tree"))?;
                    let min = shape::min_window(layout).ok_or_else(|| unreadable("minimum"))?;
                    let mut shell = tree.shell(t.window, min, own(&t.css));
                    (shell.max_width, shell.max_height) =
                        shape::max_size(t.window, min, &layout.size_hints);
                    shell
                }
                Some(name) => {
                    let popup = layout.popups.get(name).ok_or_else(|| unreadable("popup"))?;
                    let frame = crate::skin_tree::popup::frame(popup, layout.min_height);
                    let min = shape::min_window(&frame).ok_or_else(|| unreadable("minimum"))?;
                    popup.shell(t.window, name, min, own(&t.css))
                }
            };
            shell.menus = menus.clone();
            Ok(shell)
        })
        .collect()
}

/// The pack's own link (plan SK10): the first `LaunchUrl` a node of its
/// trees runs (Total Chaos's logo, `AppLaunchUrl(http://www.totalchaos.org.uk/)`),
/// in window then document order; the Help menu serves it as
/// "<pack>'s website" ([`MenuRegistry::for_pack`]).
fn pack_link(m: &SkinManifest) -> Option<PackUrl> {
    fn find(nodes: &[SkinNode]) -> Option<PackUrl> {
        nodes.iter().find_map(|n| {
            n.triggers
                .iter()
                .find_map(|t| match &t.run {
                    SkinAction::LaunchUrl(u) => Some(u.clone()),
                    _ => None,
                })
                .or_else(|| find(&n.children))
        })
    }
    m.windows.values().find_map(|l| {
        l.tree
            .as_ref()
            .and_then(|t| find(&t.root))
            .or_else(|| l.popups.values().find_map(|p| find(&p.tree.root)))
    })
}

/// Every distinct image href under `base` that `css` draws, in order: read
/// from the emitted stylesheet itself, so the list can never miss a rule
/// (the region, title, label and frame emitters each write `url("…")`).
fn sheet_images(css: &str, base: &str) -> Vec<String> {
    let prefix = format!("{base}/");
    let mut seen = std::collections::BTreeSet::new();
    css.split("url(\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .filter(|href| href.starts_with(&prefix) && seen.insert(*href))
        .map(str::to_owned)
        .collect()
}

/// The window frames of the canonical package `bytes` served at `address`: each window kind's
/// layout with its tiles' masks (SK0 S3), laid as [`crate::conform::served`] lays it.
///
/// # Errors
/// The package or its manifest does not read, or a tile's image does not
/// decode at its declared size (never for a package the helper returned).
pub fn frames(bytes: &[u8], address: &str) -> Result<Vec<SkinWindowFrame>, SkinError> {
    let p = package::read(bytes)?;
    let (m, _) = crate::conform::served(bytes, &p, address)?;
    let images = files(&p);
    m.windows
        .iter()
        .map(|(window, layout)| {
            let masks = shape::of_layout("windows", layout, &images)?
                .into_iter()
                .map(|mask| SkinTileMask {
                    width: mask.width(),
                    height: mask.height(),
                    bits: mask.bits().to_vec(),
                    faint: mask.faint().to_vec(),
                })
                .collect();
            Ok(SkinWindowFrame {
                window: *window,
                layout: layout.clone(),
                masks,
            })
        })
        .collect()
}

/// `file` of the canonical package `bytes` at `address`; `Ok(None)` for
/// an image the package does not hold.
///
/// # Errors
/// What [`stylesheet`] returns.
pub fn body(bytes: &[u8], address: &str, file: File<'_>) -> Result<Option<Body>, SkinError> {
    match file {
        File::Stylesheet => Ok(Some(Body {
            content_type: "text/css; charset=utf-8",
            bytes: stylesheet(bytes, address)?.css.into_bytes(),
        })),
        File::Image { hash, ext } => {
            let bytes = image::named(bytes, address, hash, ext)?;
            Ok(bytes.map(|bytes| Body {
                content_type: if ext == "gif" {
                    "image/gif"
                } else {
                    "image/png"
                },
                bytes,
            }))
        }
    }
}

#[cfg(test)]
mod tests;
