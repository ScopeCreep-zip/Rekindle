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
                label.is_some_and(|l| l.text == BRAND && l.glyph.is_some() && l.cover.is_some()),
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
