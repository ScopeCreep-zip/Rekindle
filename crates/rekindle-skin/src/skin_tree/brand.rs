//! Rekindle's brand in what the Host serves (owner 2026-11-23, "1:1 Xfire
//! look, Rekindle brand"; design `br` item 2). The packs keep Xfire's look,
//! but nothing served names or links Xfire:
//!
//! - every [`NodeRole::Logo`] node and the decor that bakes Xfire's mark
//!   (Xfire Classic's `TheX` and `TheXCover`, Alloy's popup `InfoLogo`,
//!   Onyx's `Logo2`) carries a label `"Rekindle"` covering its whole art box
//!   with the brand glyph ([`glyph`]: the wordmark in a box more than twice
//!   as wide as tall, the mark otherwise), so the shell draws Rekindle's
//!   mark in that box. The cover is the node's whole box, so the node
//!   serves no art in any state (its `art` is dropped): the pack's Xfire
//!   logo never reaches the webview, and the shell draws the glyph on a
//!   transparent cover over the parent's art. A branded node inside a
//!   branded ancestor (Classic's `TheX` and `TheXCover` in its wordmark
//!   `XfireLogo`) serves no art and no label: the ancestor's cover is
//!   already over its box. A pack's own emblem (Space Cowboy's
//!   `SCLogo`, Total Chaos's "T.C" `Title`) is not Xfire's and stays;
//! - a `LaunchUrl` trigger whose URL is Xfire's ([`is_xfire_url`]) is
//!   dropped, the node's other triggers kept (Onyx's `Logo2` becomes inert
//!   art), so the Help menu's "<pack>'s website" (`serve::pack_link`) never
//!   resolves to xfire.com;
//! - the served pack name and description drop the word "Xfire"
//!   ([`name`], [`description`]): Xfire Classic is served as "Classic", the
//!   sound packs as "Classic sound pack" and "Default sound pack". Credits
//!   and licence text are attribution and stay as written ("Xfire, Inc."
//!   among the authors), but a credits URL that is Xfire's is not served
//!   ([`credits`], owner 2026-11-23: pack links never point at xfire.com).
//!
//! The title strip's logo (`TitleRole::Logo`) is drawn from the same glyph:
//! `shape::title` only places its box, and `serve::stylesheet` names the
//! glyph for the first strip logo (`SkinApplied::title_logo`), which the
//! shell draws inline in that box; the pack's `title-logo.png` is never
//! served.

use rekindle_types::dto::skin::{SkinCredits, SkinManifest, WindowLayout};
use rekindle_types::dto::skin_action::SkinAction;
use rekindle_types::dto::skin_tree::{NodeGlyph, NodeLabel, NodeRole, SkinNode, SkinTree};

use super::titles::{each_laid, laid_window, place};

/// The brand's name: a branded node's label and accessible name.
pub const BRAND: &str = "Rekindle";

/// The decor ids that bake Xfire's mark (see the module doc).
const XFIRE_DECOR: [&str; 4] = ["thex", "thexcover", "infologo", "logo2"];

/// The brand glyph for a `width x height` box (lead, from the finished art
/// `src/assets/brand/`, wordmark ~4.38:1): the wordmark (flame and word)
/// when the box is at least 15 px tall and more than twice as wide as
/// tall (the word is unreadable at 9 px); else the small mark when it is
/// at most 16 px tall; else the full mark.
#[must_use]
pub fn glyph(width: u16, height: u16) -> NodeGlyph {
    if height >= 15 && u32::from(width) > 2 * u32::from(height) {
        NodeGlyph::RekindleWordmark
    } else if height <= 16 {
        NodeGlyph::RekindleMarkSmall
    } else {
        NodeGlyph::Rekindle
    }
}

/// The brand glyph of the first pack title strip's logo box, Main before
/// Chat ([`glyph`] of its size); `None` when no strip lays a logo. The
/// shell draws it inline (`SkinApplied::title_logo`); the sheet only
/// places the box, so the pack's Xfire logo is never served.
#[must_use]
pub fn title_logo(m: &SkinManifest) -> Option<NodeGlyph> {
    m.windows
        .values()
        .filter_map(|w| w.title.as_ref())
        .flat_map(|t| &t.parts)
        .find(|p| p.role == rekindle_types::dto::skin_title::TitleRole::Logo)
        .map(|p| glyph(p.width, p.height))
}

/// The glyph each built-in logo box serves (lead pin): Total Chaos's
/// 23 x 9 and the 16 x 16 logos the small mark, the wide title logos the
/// wordmark, Alloy's 67 x 40 the full mark.
#[cfg(test)]
#[test]
fn the_logo_boxes_take_their_pinned_glyphs() {
    use NodeGlyph as G;
    for (w, h, want) in [
        (23, 9, G::RekindleMarkSmall),
        (16, 16, G::RekindleMarkSmall),
        (48, 23, G::RekindleWordmark),
        (67, 19, G::RekindleWordmark),
        (87, 19, G::RekindleWordmark),
        (89, 21, G::RekindleWordmark),
        (121, 37, G::RekindleWordmark),
        (194, 44, G::RekindleWordmark),
        (67, 40, G::Rekindle),
    ] {
        assert_eq!(glyph(w, h), want, "{w} x {h}");
    }
}

/// The lowercase host of `url` (scheme optional, user and port dropped).
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn is_xfire_host(host: &str) -> bool {
    host == "xfire.com" || host.ends_with(".xfire.com")
}

/// Whether `url` is Xfire's: its host `xfire.com` or a subdomain, or a
/// Wayback Machine copy of such a URL (`web.archive.org/web/<stamp>/<url>`).
#[must_use]
pub fn is_xfire_url(url: &str) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    if is_xfire_host(&host) {
        return true;
    }
    if !(host == "archive.org" || host.ends_with(".archive.org")) {
        return false;
    }
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.split_once('/').map_or("", |(_, p)| p);
    let mut parts = path.splitn(3, '/');
    parts.next() == Some("web")
        && parts.next().is_some()
        && parts
            .next()
            .and_then(host_of)
            .is_some_and(|h| is_xfire_host(&h))
}

/// `text` without the word "Xfire" (or "Xfire's"), any case.
fn without_xfire(text: &str) -> String {
    text.split_whitespace()
        .filter(|w| {
            let stem = w.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
            let stem = stem.strip_suffix("'s").unwrap_or(stem);
            !stem.eq_ignore_ascii_case("xfire")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A pack's served name: "Xfire" alone is "Classic", else the name without
/// the word.
#[must_use]
pub fn name(name: &str) -> String {
    let plain = without_xfire(name);
    if plain.is_empty() {
        "Classic".to_owned()
    } else {
        plain
    }
}

/// A pack's served description, without the word "Xfire" ("Xfire's own
/// sound pack" is "The original sound pack").
#[must_use]
pub fn description(text: &str) -> String {
    without_xfire(&text.replace("Xfire's own sound pack", "The original sound pack"))
}

/// A pack's served credits: as written, less a `url` that is Xfire's
/// ([`is_xfire_url`]); the authors ("Xfire, Inc." where the pack credits
/// it), contributors and licence stay.
#[must_use]
pub fn credits(credits: Option<SkinCredits>) -> Option<SkinCredits> {
    credits.map(|c| SkinCredits {
        url: c.url.filter(|u| !is_xfire_url(u)),
        ..c
    })
}

/// The served name, description and credits of `manifest` (see [`name`],
/// [`credits`]).
pub fn texts(manifest: &mut SkinManifest) {
    manifest.name = name(&manifest.name);
    manifest.description = description(&manifest.description);
    manifest.credits = credits(manifest.credits.take());
}

/// Brands `layout`'s tree and every popup tree, each laid in its window's
/// served minimum (see the module doc).
pub fn nodes(layout: &mut WindowLayout) {
    each_laid(layout, tree);
}

fn tree(tree: &mut SkinTree, at: (i32, i32)) {
    walk(&mut tree.root, laid_window(tree.outsets, at), false);
}

pub(super) fn branded(n: &SkinNode) -> bool {
    n.role == NodeRole::Logo || XFIRE_DECOR.iter().any(|d| n.id.eq_ignore_ascii_case(d))
}

/// Brands `nodes` laid in `parent`; `inside` when a branded ancestor's
/// cover is already over them.
fn walk(nodes: &mut [SkinNode], parent: [i32; 4], inside: bool) {
    for n in nodes {
        n.triggers
            .retain(|t| !matches!(&t.run, SkinAction::LaunchUrl(u) if is_xfire_url(&u.url)));
        let b = place(n, parent);
        let mine = branded(n);
        if mine {
            // The cover is the whole box: the pack's (Xfire) art is never
            // served, in any state.
            n.art = None;
            n.label = if inside {
                None
            } else {
                match (u16::try_from(b[2]), u16::try_from(b[3])) {
                    (Ok(w), Ok(h)) => Some(NodeLabel {
                        text: BRAND.to_owned(),
                        cover: Some([0, 0, w, h]),
                        glyph: Some(glyph(w, h)),
                        style: None,
                    }),
                    _ => n.label.take(),
                }
            };
        }
        walk(&mut n.children, b, inside || mine);
    }
}

#[cfg(test)]
#[path = "brand_tests.rs"]
mod tests;
