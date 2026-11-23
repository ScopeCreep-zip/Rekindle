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
use rekindle_types::dto::skin_action::{MenuRegistry, PackUrl, SkinAction};
use rekindle_types::dto::skin_library::{SkinPaletteColour, SkinTileMask, SkinWindowFrame};
use rekindle_types::dto::skin_tree::{NodeGlyph, SkinNode, SkinShell};

use crate::generate::tui_colours;
use crate::output::tokens_css;
use crate::package::{self, Package};
use crate::regions::css::skin_css;
use crate::regions::labels::label_css;
use crate::regions::still;
use crate::regions::title::title_css;
use crate::registry::{resolve, Base, TokenEntry, ENTRIES};
use crate::shape;
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
    /// [`sheet_images`] of the shared rules: every image href any window's sheet may draw (SK4).
    pub images: Vec<String>,
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

fn files(p: &Package<'_>) -> BTreeMap<String, Vec<u8>> {
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
    let m = manifest::served(&p)?;
    let (v, resolved) = active_resolved(&p, &m)?;
    let mut css = tokens_css(&resolved).ok_or_else(|| unreadable("tokens do not emit"))?;
    let base = format!("{}/{address}", origin());
    let images = files(&p);
    css.push_str(&skin_css(&m, &images, &base).ok_or_else(|| unreadable("regions"))?);
    css.push_str(&title_css(&m, &images, &resolved));
    css.push_str(&label_css(&m, &images, &resolved));
    let (frame, trees) =
        shape::frame_parts(&m, &images, &base).ok_or_else(|| unreadable("frame"))?;
    let mut shared_css = css.clone();
    shared_css.push_str(&frame);
    let shared = sheet_images(&shared_css, &base);
    css.push_str(&shape::layered(&frame, &trees));
    Ok(Stylesheet {
        base: v.base,
        shells: shells(&m, &trees, &shared, &base)?,
        images: shared,
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
                    tree.shell(t.window, min, own(&t.css))
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

/// The window frames of the canonical package `bytes`: each window kind's
/// layout with its tiles' masks decoded from the package's images (plan
/// SK0 S3, what the shell cuts its windows by).
///
/// # Errors
/// The package or its manifest does not read, or a tile's image does not
/// decode at its declared size (never for a package the helper returned).
pub fn frames(bytes: &[u8]) -> Result<Vec<SkinWindowFrame>, SkinError> {
    let p = package::read(bytes)?;
    let m = manifest::served(&p)?;
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
    fn every_builtin_lists_each_image_its_stylesheet_draws_once() {
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap();
            let a = c.address();
            let applied = stylesheet(&c.bytes, &a).unwrap();
            assert!(!applied.images.is_empty(), "{}", pack.id);
            let shared: std::collections::BTreeSet<&str> =
                applied.images.iter().map(String::as_str).collect();
            assert_eq!(shared.len(), applied.images.len(), "{} twice", pack.id);
            let mut listed = shared.clone();
            for shell in &applied.shells {
                let own: std::collections::BTreeSet<&str> =
                    shell.images.iter().map(String::as_str).collect();
                assert_eq!(own.len(), shell.images.len(), "{} twice", pack.id);
                assert!(own.is_disjoint(&shared), "{} shares tree art", pack.id);
                listed.extend(own);
            }
            for href in &listed {
                let path = href.strip_prefix(origin()).expect("under the origin");
                assert!(parse_path(path).is_some(), "{href} is no served file");
                assert!(applied.css.contains(&format!("url(\"{href}\")")), "{href}");
            }
            for drawn in applied.css.split("url(\"").skip(1) {
                let href = drawn.split('"').next().unwrap();
                assert!(listed.contains(href), "{} draws {href} unlisted", pack.id);
            }
        }
    }

    /// Plan SK4: the shared list a window without a tree (Settings)
    /// decodes holds no tree art, each tree's art is on its own shell, and
    /// no image is lost (the union is the whole sheet's list).
    #[test]
    fn a_trees_art_is_served_on_its_shell_and_none_is_lost() {
        let symbiosis = builtin::PACKS.iter().find(|p| p.id == "symbiosis").unwrap();
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap();
            let a = c.address();
            let applied = stylesheet(&c.bytes, &a).unwrap();
            let whole: std::collections::BTreeSet<String> =
                sheet_images(&applied.css, &format!("{}/{a}", origin()))
                    .into_iter()
                    .collect();
            let mut union: std::collections::BTreeSet<String> =
                applied.images.iter().cloned().collect();
            for shell in &applied.shells {
                union.extend(shell.images.iter().cloned());
            }
            assert_eq!(union, whole, "{}", pack.id);
            let p = package::read(&c.bytes).unwrap();
            let m = manifest::served(&p).unwrap();
            let base = format!("{}/{a}", origin());
            let (_, trees) = shape::frame_parts(&m, &files(&p), &base).unwrap();
            for href in &applied.images {
                let outside = applied.css.matches(&format!("url(\"{href}\")")).count()
                    > trees
                        .iter()
                        .map(|t| t.css.matches(&format!("url(\"{href}\")")).count())
                        .sum::<usize>();
                assert!(outside, "{}: {href} is tree art only", pack.id);
            }
            if pack.id == symbiosis.id {
                let art: usize = applied.shells.iter().map(|s| s.images.len()).sum();
                assert!(art > applied.images.len(), "{}", pack.id);
            }
        }
    }

    /// Plan SK4: a tree-laid window's shell serves its minimum with the
    /// frame's outsets, the value the desktop's fit applies
    /// ([`shape::min_window`]); Symbiosis's main window is 368 wide.
    #[test]
    fn a_shell_serves_its_windows_minimum_with_the_outsets() {
        let pack = builtin::PACKS.iter().find(|p| p.id == "symbiosis").unwrap();
        let c = validate::run(pack.bytes).unwrap();
        let applied = stylesheet(&c.bytes, &c.address()).unwrap();
        let p = package::read(&c.bytes).unwrap();
        let m = manifest::served(&p).unwrap();
        let main = &m.windows[&rekindle_types::dto::skin::SkinWindow::Main];
        let shell = applied
            .shells
            .iter()
            .find(|s| s.window == rekindle_types::dto::skin::SkinWindow::Main && s.popup.is_none())
            .unwrap();
        assert_eq!(shell.min_width, 368);
        assert_eq!(
            Some((shell.min_width, shell.min_height)),
            shape::min_window(main)
        );
        let o = main.outsets;
        assert_eq!(shell.min_width, main.min_width + o.left + o.right);
        assert_eq!(shell.min_height, main.min_height + o.top + o.bottom);
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

    /// Every built-in serves a stylesheet with balanced braces,
    /// parentheses and strings and no comment. Its time is held in its
    /// own test binary (`tests/serve_budget.rs`), and the title's column
    /// work, the regression that budget caught, by an operation count
    /// (`regions/title_tests.rs`).
    #[test]
    fn every_builtin_validates_and_serves_a_balanced_sheet() {
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", pack.id));
            let sheet = stylesheet(&c.bytes, &c.address()).expect("a stylesheet");
            let count = |ch: char| sheet.css.matches(ch).count();
            assert_eq!(count('{'), count('}'), "{}: braces", pack.id);
            assert_eq!(count('('), count(')'), "{}: parentheses", pack.id);
            assert_eq!(count('"') % 2, 0, "{}: an open string", pack.id);
            assert!(!sheet.css.contains("/*"), "{}: a comment", pack.id);
        }
    }

    /// Design §2.6: a pressed menu's items come from the served shell, so
    /// every shell a built-in serves carries Rekindle's menu registry for
    /// that pack (plan SK10: with "<pack>'s website" in Help when the pack
    /// declares a link, as Total Chaos's logo does; Rekindle's menus
    /// unchanged otherwise).
    #[test]
    fn a_served_shell_carries_the_registrys_menus() {
        use rekindle_types::dto::skin_action::EngineMenu;
        let registry = MenuRegistry::rekindle();
        assert!(!registry.menus.is_empty());
        let mut linked = Vec::new();
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap();
            let m = manifest::served(&package::read(&c.bytes).unwrap()).unwrap();
            let want = MenuRegistry::for_pack(&m.name, pack_link(&m).as_ref());
            let sheet = stylesheet(&c.bytes, &c.address()).unwrap();
            assert!(!sheet.shells.is_empty(), "{}", pack.id);
            for shell in &sheet.shells {
                assert_eq!(shell.menus, want, "{} {:?}", pack.id, shell.window);
            }
            let help = want.items(EngineMenu::Help);
            let site = help.iter().find(|i| i.label.ends_with("'s website"));
            if let Some(site) = site {
                linked.push((pack.id, site.label.clone(), site.run.clone()));
            } else {
                assert_eq!(want, registry, "{}: a menu without a link", pack.id);
            }
        }
        // The real set (design `br`): Total Chaos's `TCLink` and Space
        // Cowboy's `SCLogo`; the packs whose logo ran xfire.com serve none.
        let mut ids: Vec<&str> = linked.iter().map(|(id, ..)| *id).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["spacecowboy", "total-chaos"]);
        for (id, _, run) in &linked {
            let SkinAction::LaunchUrl(u) = run else {
                panic!("{id}: the website item runs {run:?}");
            };
            assert!(!crate::skin_tree::brand::is_xfire_url(&u.url), "{id}");
        }
        let chaos = linked.iter().find(|(id, ..)| *id == "total-chaos");
        let (_, label, run) = chaos.expect("Total Chaos's Help menu serves its website");
        assert_eq!(label, "Total Chaos's website");
        assert_eq!(
            *run,
            SkinAction::LaunchUrl(PackUrl {
                url: "http://www.totalchaos.org.uk/".to_owned()
            })
        );
    }

    #[test]
    fn only_built_ins_are_immutable() {
        assert!(Cache::Immutable.header().contains("immutable"));
        assert_eq!(Cache::NoStore.header(), "no-store");
    }
}
