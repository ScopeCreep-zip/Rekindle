use super::*;
use crate::validate;
use rekindle_types::dto::skin::{PopupEdge, SkinWindow};

#[test]
fn every_pack_is_canonical_and_the_default_is_one() {
    for pack in &PACKS {
        let c = validate::run(pack.bytes).unwrap();
        // The committed pack is the pipeline's fixed point, so what is
        // served is byte for byte what was imported.
        assert_eq!(c.bytes, pack.bytes, "builtin:{}", pack.id);
    }
    assert_eq!(find(DEFAULT).map(|(i, _)| i), Some(0));
    assert!(find("builtin:xfire-classic").is_some());
    assert!(find("builtin:combat").is_some());
    assert!(find("builtin:nope").is_none());
    assert!(find("symbiosis").is_none());
    assert!(find("pkg:symbiosis").is_none());
    for (i, pack) in PACKS.iter().enumerate() {
        let address = validate::run(pack.bytes).unwrap().address();
        assert_eq!(by_address(&address).map(|(j, _)| j), Some(i));
    }
    assert!(by_address(&"0".repeat(64)).is_none());
}

/// Xfire's own packs, first in [`PACKS`]; the rest are fan skins.
const XFIRE_OWN: usize = 3;

/// Plan E7.7.3d: Xfire's own built-ins credit Xfire, Inc. and link
/// their source; every fan built-in credits the authors its own files
/// name, never Xfire.
#[test]
fn every_pack_credits_its_authors() {
    for (i, pack) in PACKS.iter().enumerate() {
        let m = crate::manifest::parse(&crate::package::read(pack.bytes).unwrap()).unwrap();
        let c = m
            .credits
            .unwrap_or_else(|| panic!("builtin:{} has no credits", pack.id));
        if i < XFIRE_OWN {
            assert_eq!(c.authors, ["Xfire, Inc."], "builtin:{}", pack.id);
            assert!(c.url.is_some(), "builtin:{}", pack.id);
        } else {
            assert!(!c.authors.is_empty(), "builtin:{}", pack.id);
            assert!(
                !c.authors.iter().any(|a| a.contains("Xfire")),
                "builtin:{}",
                pack.id
            );
        }
    }
}

/// Plan E7.7.3e: a fan built-in carries its author's art, at least one
/// image region, never only a palette.
#[test]
fn every_fan_pack_carries_its_art() {
    for pack in &PACKS[XFIRE_OWN..] {
        let m = crate::manifest::parse(&crate::package::read(pack.bytes).unwrap()).unwrap();
        assert!(!m.regions.is_empty(), "builtin:{} is colour-only", pack.id);
    }
}

/// The main window's popups of `pack`: name → (anchor edge, offset,
/// width, min width), and the names of the main tree's top-level nodes.
fn popups_of(
    pack: &Builtin,
) -> (
    std::collections::BTreeMap<String, (PopupEdge, i16, u16, u16)>,
    Vec<String>,
) {
    let m = crate::manifest::parse(&crate::package::read(pack.bytes).unwrap()).unwrap();
    let main = &m.windows[&SkinWindow::Main];
    let popups = main
        .popups
        .iter()
        .map(|(n, p)| {
            (
                n.clone(),
                (p.anchor.edge, p.anchor.offset, p.width, p.min_width),
            )
        })
        .collect();
    let top = main
        .tree
        .iter()
        .flat_map(|t| t.root.iter().map(|n| n.id.clone()))
        .collect();
    (popups, top)
}

/// Plan SK2-P1d (design §2.3): every built-in's main window serves the
/// popups its tree launches; Symbiosis's info popup is 196 wide at
/// `X="MainWindow.Right-13"` (`Popups.xml:10`), Classic's 315 wide
/// (173 at its smallest) at `X="MainRightOuterEdge.RIGHT"`
/// (`xfirepopups.xml:2`), and StealthX's rail exists only as a popup.
#[test]
fn every_pack_serves_its_popups() {
    for pack in &PACKS {
        let (popups, _) = popups_of(pack);
        assert!(!popups.is_empty(), "builtin:{} serves no popup", pack.id);
    }
    let pack = |id: &str| find(&format!("builtin:{id}")).unwrap().1;
    let (sym, _) = popups_of(pack("symbiosis"));
    assert_eq!(sym["MainInfoViewPopup"], (PopupEdge::Right, -13, 196, 196));
    let (classic, _) = popups_of(pack("xfire-classic"));
    assert_eq!(
        classic["MainInfoViewPopup"],
        (PopupEdge::Right, -13, 315, 173)
    );
    let (stealth, top) = popups_of(pack("stealthx"));
    assert!(stealth.contains_key("MainMenuPopup"));
    assert!(!top.iter().any(|n| n == "MainMenuPopup"), "{top:?}");
}
