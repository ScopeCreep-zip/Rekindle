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
    SkinBase, SkinContrast, SkinError, SkinManifest, SkinRule, SkinVariant, TokenValue,
};
use rekindle_types::dto::skin_library::{SkinPaletteColour, SkinTileMask, SkinWindowFrame};

use crate::generate::tui_colours;
use crate::output::tokens_css;
use crate::package::{self, Package};
use crate::regions::css::skin_css;
use crate::regions::labels::label_css;
use crate::regions::still;
use crate::regions::title::title_css;
use crate::registry::{resolve, Base, TokenEntry, ENTRIES};
use crate::shape::{self, frame_css};
use crate::{manifest, tokens};

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
    /// [`crate::generate::tui_colours`] of the active variant.
    pub palette: Vec<SkinPaletteColour>,
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

fn files(p: &Package<'_>) -> BTreeMap<String, Vec<u8>> {
    p.files().map(|(n, b)| (n.to_owned(), b.to_vec())).collect()
}

/// The stylesheet of the canonical package `bytes` at `address`: the
/// active variant's resolved tokens, then the region rules with images
/// under `{origin}/{address}`, then the window frames
/// ([`frame_css`], plan SK0 S2).
///
/// # Errors
/// The package, its manifest or its tokens do not read, or a region does
/// not emit (never for a package the helper returned).
pub fn stylesheet(bytes: &[u8], address: &str) -> Result<Stylesheet, SkinError> {
    let p = package::read(bytes)?;
    let m = manifest::parse(&p)?;
    let (v, resolved) = active_resolved(&p, &m)?;
    let mut css = tokens_css(&resolved).ok_or_else(|| unreadable("tokens do not emit"))?;
    let base = format!("{}/{address}", origin());
    let images = files(&p);
    css.push_str(&skin_css(&m, &images, &base).ok_or_else(|| unreadable("regions"))?);
    css.push_str(&title_css(&m, &images, &resolved));
    css.push_str(&label_css(&m, &images, &resolved));
    css.push_str(&frame_css(&m, &images, &base).ok_or_else(|| unreadable("frame"))?);
    Ok(Stylesheet {
        base: v.base,
        css,
        regions: m.regions.keys().cloned().collect(),
        palette: tui_colours(&resolved),
    })
}

/// The window frames of the canonical package `bytes`: each window kind's
/// layout with its tiles' masks decoded from the package's images (plan
/// SK0 S3, what the shell cuts its windows by).
///
/// # Errors
/// The package or its manifest does not read, or a tile's image does not
/// decode at its declared size (never for a package the helper returned).
pub fn frames(bytes: &[u8]) -> Result<Vec<SkinWindowFrame>, SkinError> {
    let p = package::read(bytes)?;
    let m = manifest::parse(&p)?;
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
            let p = package::read(bytes)?;
            let found = p
                .files()
                .find(|(name, data)| {
                    name.rsplit_once('.').map(|(_, e)| e) == Some(ext)
                        && blake3::hash(data).to_hex().as_str() == hash
                })
                .map(|(_, data)| data.to_vec());
            let bytes = match found {
                Some(bytes) => Some(bytes),
                None if ext == "png" => still_named(&p, hash)?,
                None => None,
            };
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

/// The still (`skin_css`'s reduced-motion first frame, [`still::stills`])
/// of the package's animated region image whose PNG has address `hash`.
fn still_named(p: &Package<'_>, hash: &str) -> Result<Option<Vec<u8>>, SkinError> {
    let m = manifest::parse(p)?;
    let stills = still::stills(&m, &files(p)).map_err(|e| unreadable(&e.to_string()))?;
    Ok(stills
        .into_values()
        .find(|png| blake3::hash(png).to_hex().as_str() == hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{builtin, validate};

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn only_a_package_address_and_one_file_parse() {
        assert_eq!(
            parse_path(&format!("/{H}/tokens.css")),
            Some((H, File::Stylesheet))
        );
        assert_eq!(
            parse_path(&format!("/{H}/{H}.gif")),
            Some((
                H,
                File::Image {
                    hash: H,
                    ext: "gif"
                }
            ))
        );
        for bad in [
            format!("/{H}/../{H}/tokens.css"),
            format!("/{H}/x/tokens.css"),
            format!("/../{H}/tokens.css"),
            format!("/{H}/%2e%2e/tokens.css"),
            format!("/{H}/{H}.svg"),
            format!("/{H}/{H}.png.png"),
            format!("/{}/tokens.css", H.to_uppercase()),
            format!("{H}/tokens.css"),
            format!("/{H}/manifest.json"),
            format!("/{H}"),
            String::new(),
        ] {
            assert_eq!(parse_path(&bad), None, "{bad}");
        }
    }

    #[test]
    fn the_builtin_serves_its_stylesheet_and_its_images_by_address() {
        let c = validate::run(builtin::PACKS[0].bytes).unwrap();
        let a = c.address();
        let applied = stylesheet(&c.bytes, &a).unwrap();
        assert!(applied.css.starts_with("@layer skin {"));
        let prefix = format!("{}/{a}/", origin());
        let href = applied
            .css
            .split("url(\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .expect("a region image");
        let name = href
            .strip_prefix(&prefix)
            .expect("served under the package");
        let path = format!("/{a}/{name}");
        let (_, file) = parse_path(&path).unwrap();
        let body = body(&c.bytes, &a, file).unwrap().expect("the image");
        assert!(body.content_type.starts_with("image/"));
        let missing = File::Image {
            hash: H,
            ext: "png",
        };
        assert_eq!(super::body(&c.bytes, &a, missing).unwrap(), None);
    }

    /// The time one window's stylesheet may take in a debug build: the
    /// Host answers `SkinGet` and every `tokens.css` fetch with it, every
    /// window at once on a skin change, inside the client's 5 s
    /// `DEFAULT_TIMEOUT` (`rekindle-client` `client.rs`). Measured at
    /// e777t: at most about 0.6 s; before, Xfire Classic took 2.8 s and
    /// xSLIMFIRE 3.6 s (the title's per-column walk over every window
    /// width), so the windows kept the previous skin and the skin list
    /// came back empty.
    const SERVE_BUDGET: std::time::Duration = std::time::Duration::from_millis(1500);

    #[test]
    fn every_builtin_validates_and_serves_a_balanced_sheet_in_time() {
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", pack.id));
            let started = std::time::Instant::now();
            let sheet = stylesheet(&c.bytes, &c.address()).expect("a stylesheet");
            let took = started.elapsed();
            assert!(took < SERVE_BUDGET, "{} served in {took:?}", pack.id);
            let count = |ch: char| sheet.css.matches(ch).count();
            assert_eq!(count('{'), count('}'), "{}: braces", pack.id);
            assert_eq!(count('('), count(')'), "{}: parentheses", pack.id);
            assert_eq!(count('"') % 2, 0, "{}: an open string", pack.id);
            assert!(!sheet.css.contains("/*"), "{}: a comment", pack.id);
        }
    }

    #[test]
    fn only_built_ins_are_immutable() {
        assert!(Cache::Immutable.header().contains("immutable"));
        assert_eq!(Cache::NoStore.header(), "no-store");
    }
}
