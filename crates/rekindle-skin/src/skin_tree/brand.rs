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

fn branded(n: &SkinNode) -> bool {
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
                    }),
                    _ => n.label.take(),
                }
            };
        }
        walk(&mut n.children, b, inside || mine);
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::dto::skin_action::SkinAction;
    use rekindle_types::dto::skin_tree::{NodeGlyph, NodeRole, SkinNode};

    use super::{branded, glyph, is_xfire_url, name, BRAND};
    use crate::{builtin, manifest, package, validate};

    #[test]
    fn xfire_urls_and_their_archive_copies_are_xfire() {
        for url in [
            "http://www.xfire.com/",
            "https://XFIRE.com/skins",
            "xfire.com",
            "http://user@classic.xfire.com:80/x",
            "http://web.archive.org/web/2006/http://www.xfire.com/",
        ] {
            assert!(is_xfire_url(url), "{url}");
        }
        for url in [
            "http://www.totalchaos.org.uk/",
            "http://notxfire.com/",
            "http://web.archive.org/web/2006/http://www.totalchaos.org.uk/",
            "http://xfire.com.example.org/",
        ] {
            assert!(!is_xfire_url(url), "{url}");
        }
    }

    #[test]
    fn served_names_drop_xfire() {
        assert_eq!(name("Xfire"), "Classic");
        assert_eq!(name("Xfire Classic sound pack"), "Classic sound pack");
        assert_eq!(name("Xfire Default sound pack"), "Default sound pack");
        assert_eq!(name("xSLIMFIRE"), "xSLIMFIRE");
        assert_eq!(glyph(87, 19), NodeGlyph::RekindleWordmark);
        assert_eq!(glyph(16, 16), NodeGlyph::RekindleMarkSmall);
    }

    fn check(nodes: &[SkinNode], id: &str) {
        for n in nodes {
            if n.role == NodeRole::Logo {
                let label = n.label.as_ref();
                assert!(
                    label
                        .is_some_and(|l| l.text == BRAND && l.glyph.is_some() && l.cover.is_some()),
                    "{id}: logo {} without the brand glyph",
                    n.id
                );
            }
            for t in &n.triggers {
                if let SkinAction::LaunchUrl(u) = &t.run {
                    assert!(
                        !u.url.to_ascii_lowercase().contains("xfire"),
                        "{id}: {} links {}",
                        n.id,
                        u.url
                    );
                }
            }
            check(&n.children, id);
        }
    }

    /// Every built-in's served trees: no logo without the brand glyph, no
    /// link to Xfire; its served name and description never say Xfire.
    #[test]
    fn no_builtin_serves_xfire_branding() {
        let mut logos = 0;
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap();
            let m = manifest::served(&package::read(&c.bytes).unwrap()).unwrap();
            for text in [&m.name, &m.description] {
                assert!(
                    !text.to_ascii_lowercase().contains("xfire"),
                    "{}: {text}",
                    pack.id
                );
            }
            for layout in m.windows.values() {
                let trees = layout
                    .tree
                    .iter()
                    .chain(layout.popups.values().map(|p| &p.tree));
                for t in trees {
                    logos += count_logos(&t.root);
                    check(&t.root, pack.id);
                }
            }
        }
        assert!(logos > 0, "the built-ins serve logo nodes");
    }

    fn served_of(id: &str) -> rekindle_types::dto::skin::SkinManifest {
        let pack = builtin::PACKS.iter().find(|p| p.id == id).expect(id);
        let c = validate::run(pack.bytes).unwrap();
        manifest::served(&package::read(&c.bytes).unwrap()).unwrap()
    }

    /// The title strip's logo is the brand glyph its box takes (the
    /// shell draws it inline): Symbiosis's 121 x 37 the wordmark, Onyx's
    /// 16 x 16 the small mark, Combat's strip none.
    #[test]
    fn the_title_logo_takes_its_boxs_glyph() {
        assert_eq!(
            super::title_logo(&served_of("symbiosis")),
            Some(NodeGlyph::RekindleWordmark)
        );
        assert_eq!(
            super::title_logo(&served_of("onyx")),
            Some(NodeGlyph::RekindleMarkSmall)
        );
        assert_eq!(super::title_logo(&served_of("combat")), None);
    }

    /// No served stylesheet draws a pack's logo image or a brand variable:
    /// the shell draws the served glyph itself.
    #[test]
    fn no_stylesheet_draws_a_title_logo_image() {
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap();
            let address = blake3::hash(&c.bytes).to_hex().to_string();
            let sheet = crate::serve::stylesheet(&c.bytes, &address).unwrap();
            assert!(!sheet.css.contains("title-logo"), "{}", pack.id);
            assert!(!sheet.css.contains("--rekindle-"), "{}", pack.id);
        }
    }

    /// Every branded node of `nodes` (its whole box covered), each with
    /// whether a branded ancestor holds it.
    fn branded_of<'a>(nodes: &'a [SkinNode], inside: bool, out: &mut Vec<(&'a SkinNode, bool)>) {
        for n in nodes {
            let mine = branded(n);
            if mine {
                out.push((n, inside));
            }
            branded_of(&n.children, inside || mine, out);
        }
    }

    /// The served trees of every built-in window and popup.
    fn served_trees(
        m: &rekindle_types::dto::skin::SkinManifest,
    ) -> Vec<&rekindle_types::dto::skin_tree::SkinTree> {
        m.windows
            .values()
            .flat_map(|l| l.tree.iter().chain(l.popups.values().map(|p| &p.tree)))
            .collect()
    }

    /// Fix 1 (round 26 review: Combat's flame on a white box): a branded
    /// node's cover is its whole box, so no built-in serves art for it in
    /// any state, and no served stylesheet rule of it (its own, a state's or
    /// an `::after`) draws an image; Combat's `XfireLogo` among them.
    #[test]
    fn a_whole_box_covered_branded_node_serves_no_art() {
        let mut seen = 0;
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap();
            let m = manifest::served(&package::read(&c.bytes).unwrap()).unwrap();
            let address = blake3::hash(&c.bytes).to_hex().to_string();
            let sheet = crate::serve::stylesheet(&c.bytes, &address).unwrap();
            let rules: Vec<&str> = sheet.css.split("\n  }").collect();
            for t in served_trees(&m) {
                let mut nodes = Vec::new();
                branded_of(&t.root, false, &mut nodes);
                for (n, _) in nodes {
                    seen += 1;
                    assert!(n.art.is_none(), "{}: {} serves art", pack.id, n.id);
                    let sel = format!("[data-skin-node=\"{}\"]", n.id);
                    for rule in &rules {
                        let head = rule.split('{').next().unwrap_or("");
                        if head.contains(&sel) {
                            assert!(!rule.contains("url("), "{}: {rule}", pack.id);
                        }
                    }
                }
            }
            if pack.id == "combat" {
                assert!(
                    sheet.css.contains("[data-skin-node=\"XfireLogo\"]"),
                    "combat"
                );
            }
        }
        assert!(seen > 0, "the built-ins serve branded nodes");
    }

    /// Fix 2 (round 26 review: Classic's wordmark clipped to "ekindle"): a
    /// branded node inside a branded ancestor serves no label or cover, so
    /// Classic's `TheX` and `TheXCover` lie bare under `XfireLogo`'s
    /// wordmark cover.
    #[test]
    fn a_branded_node_in_a_branded_ancestor_serves_no_cover() {
        let m = served_of("xfire-classic");
        let mut nodes = Vec::new();
        for t in served_trees(&m) {
            branded_of(&t.root, false, &mut nodes);
        }
        let inner: Vec<&str> = nodes
            .iter()
            .filter(|(_, i)| *i)
            .map(|(n, _)| n.id.as_str())
            .collect();
        assert!(
            inner.contains(&"TheX") && inner.contains(&"TheXCover"),
            "{inner:?}"
        );
        for (n, inside) in nodes {
            if inside {
                assert!(n.label.is_none(), "{} keeps {:?}", n.id, n.label);
            } else {
                assert!(
                    n.label.as_ref().is_some_and(|l| l.cover.is_some()),
                    "{}",
                    n.id
                );
            }
        }
    }

    /// Fix 3 (owner 2026-11-23): no built-in serves a credits URL that is
    /// Xfire's, and the authors still credit "Xfire, Inc." where the pack
    /// does.
    #[test]
    fn no_builtin_serves_an_xfire_credits_url() {
        let mut xfire_authored = 0;
        for pack in &builtin::PACKS {
            let c = validate::run(pack.bytes).unwrap();
            let read = package::read(&c.bytes).unwrap();
            let raw = manifest::parse(&read).unwrap().credits;
            let m = manifest::served(&read).unwrap();
            let served = m.credits.as_ref().expect(pack.id);
            if let Some(u) = &served.url {
                assert!(!is_xfire_url(u), "{}: {u}", pack.id);
            }
            let draft = crate::draft::from_package(pack.id, &c.bytes).unwrap();
            assert!(
                draft
                    .credits
                    .url
                    .as_deref()
                    .is_none_or(|u| !is_xfire_url(u)),
                "{}",
                pack.id
            );
            assert_eq!(
                raw.map(|r| r.authors),
                Some(served.authors.clone()),
                "{}",
                pack.id
            );
            xfire_authored += usize::from(served.authors.iter().any(|a| a == "Xfire, Inc."));
        }
        assert!(xfire_authored > 0, "a built-in credits Xfire, Inc.");
    }

    fn count_logos(nodes: &[SkinNode]) -> usize {
        nodes
            .iter()
            .map(|n| usize::from(n.role == NodeRole::Logo) + count_logos(&n.children))
            .sum()
    }
}
