use std::collections::BTreeMap;

use rekindle_types::dto::skin::{Insets, RegionFit, SkinBase, SkinContrast, SkinRule};
use rekindle_types::dto::skin_draft::{SkinDraft, SkinDraftRegion};

use super::*;
use crate::import::pidgin::tests::png;
use crate::validate;

fn draft(tokens: BTreeMap<String, TokenValue>) -> SkinDraft {
    SkinDraft {
        name: "My Skin".to_owned(),
        description: String::new(),
        author: "Ana".to_owned(),
        license: "CC-BY-4.0".to_owned(),
        credits: SkinCredits {
            authors: Vec::new(),
            contributors: Vec::new(),
            url: None,
            licence: OriginalLicence::NoneStated,
        },
        base: SkinBase::Dark,
        contrast: SkinContrast::Standard,
        tokens,
        from: None,
        regions: BTreeMap::new(),
    }
}

fn no_assets() -> Assets {
    Assets::default()
}

/// The accent fill's colour as text: 4.42:1 on `#0a0a0a` and worse on
/// every lighter ground (plan E7.7.0's accent split).
fn failing() -> SkinDraft {
    draft(BTreeMap::from([(
        "accent.text".to_owned(),
        color([0x16, 0x7b, 0xc0]),
    )]))
}

#[test]
fn the_defaults_pass_every_pair_and_export() {
    let r = report(&draft(BTreeMap::new())).unwrap();
    assert!(r.pairs.len() > 100, "{} pairs", r.pairs.len());
    assert!(r.pairs.iter().all(|p| p.pass && p.suggestion.is_none()));
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    assert!(r.exportable);
    assert!(r.tokens_css.starts_with("@layer skin {"));
    assert!(r.tokens.iter().any(|t| t.id == "accent.text" && !t.set));
    assert!(
        r.tokens.iter().all(|t| t.id != "accent.fg"),
        "a fixed token is listed"
    );
}

#[test]
fn a_failing_pair_blocks_export_and_its_suggestion_fixes_it() {
    let d = failing();
    let r = report(&d).unwrap();
    assert!(!r.exportable);
    let bad: Vec<_> = r.pairs.iter().filter(|p| !p.pass).collect();
    assert!(!bad.is_empty());
    let first = bad[0];
    assert_eq!(first.fg, "accent.text");
    assert!(first.ratio_x100 < first.required_x100);
    assert!(first.apca_lc_x10 < 0, "light text on a dark ground");
    let s = first
        .suggestion
        .clone()
        .expect("the walk finds a lightness");
    assert_eq!(s.token, "accent.text");
    assert!(s.ratio_x100 >= first.required_x100);
    // The author's click: the suggested colour fixes that pair.
    let mut fixed = d.clone();
    fixed.tokens.insert(s.token.clone(), color(s.rgb));
    let after = report(&fixed).unwrap();
    let same = after
        .pairs
        .iter()
        .find(|p| p.rule == first.rule && p.fg == first.fg && p.ground == first.ground)
        .unwrap();
    assert!(same.pass);
}

#[test]
fn a_failing_draft_is_refused_by_the_validator_with_the_named_ratio() {
    let built = package(&failing(), &no_assets()).unwrap();
    let err = validate::run(&built.bytes).unwrap_err();
    assert!(
        matches!(&err.rule, SkinRule::Contrast { token, measured_x100, .. }
            if token == "accent.text" && *measured_x100 < 450),
        "{err:?}"
    );
}

#[test]
fn an_exported_draft_validates_reimports_byte_identically_and_starts_a_draft() {
    let mut d = draft(BTreeMap::from([(
        "bg.panel".to_owned(),
        color([0x12, 0x14, 0x18]),
    )]));
    assert!(report(&d).unwrap().exportable);
    let built = package(&d, &no_assets()).unwrap();
    let canonical = validate::run(&built.bytes).unwrap();
    assert_eq!(
        canonical.bytes, built.bytes,
        "the export is already canonical"
    );
    assert_eq!(
        validate::run(&canonical.bytes).unwrap().bytes,
        canonical.bytes
    );
    let back = from_package("pkg:x", &canonical.bytes).unwrap();
    assert_eq!(back.tokens, d.tokens);
    assert_eq!(back.license, "CC-BY-4.0");
    assert_eq!(back.credits.authors, ["Ana"]);
    d.name = "  ".to_owned();
    assert_eq!(slug(&d.name), "draft");
    assert_eq!(slug("Neon -- Night 2"), "neon-night-2");
}

#[test]
fn an_unknown_or_fixed_token_is_refused() {
    let unknown = draft(BTreeMap::from([("no.such".to_owned(), color([1, 2, 3]))]));
    assert!(report(&unknown).is_err());
    let fixed = draft(BTreeMap::from([("accent.fg".to_owned(), color([1, 2, 3]))]));
    assert!(report(&fixed).is_err());
}

/// Owner 2026-11-18: the export states the author's licence and credits
/// the author, then the skin it came from (its authors and the licence it
/// was made under), and keeps the source's regions and their images.
#[test]
fn a_derived_export_credits_its_source_and_keeps_its_regions() {
    let (_, pack) = crate::builtin::find("builtin:symbiosis").unwrap();
    let source = crate::manifest::parse(&package::read(pack.bytes).unwrap()).unwrap();
    let mut d = from_package("builtin:symbiosis", pack.bytes).unwrap();
    let original = source.credits.clone().unwrap();
    assert_eq!(d.credits.authors, original.authors);
    assert!(matches!(d.credits.licence, OriginalLicence::Text(_)));
    d.author = "Bea".to_owned();
    d.license = "MIT".to_owned();
    let assets = Assets {
        source: Some(pack.bytes.to_vec()),
        regions: BTreeMap::new(),
    };
    let built = package(&d, &assets).unwrap();
    let canonical = validate::run(&built.bytes).unwrap();
    assert_eq!(
        validate::run(&canonical.bytes).unwrap().bytes,
        canonical.bytes
    );
    let m = crate::manifest::parse(&package::read(&canonical.bytes).unwrap()).unwrap();
    assert_eq!(m.license, SkinLicense::Spdx("MIT".to_owned()));
    let credits = m.credits.unwrap();
    assert_eq!(credits.authors[0], "Bea");
    assert_eq!(&credits.authors[1..], &original.authors[..]);
    assert_eq!(credits.licence, d.credits.licence);
    assert_eq!(
        m.regions.keys().collect::<Vec<_>>(),
        source.regions.keys().collect::<Vec<_>>()
    );
    assert!(!m.regions.is_empty(), "symbiosis paints regions");
    // A licence that is not an SPDX expression is stated as its text.
    assert!(matches!(
        license_of("Free for non-commercial use."),
        SkinLicense::Text(_)
    ));
}

fn with_region(insets: Insets) -> (SkinDraft, Assets) {
    let mut d = draft(BTreeMap::new());
    d.regions.insert(
        "scrollbar.track".to_owned(),
        SkinDraftRegion {
            path: "/tmp/track.png".to_owned(),
            fit: RegionFit::NineSlice(insets),
            text_backing: None,
        },
    );
    let assets = Assets {
        source: None,
        regions: BTreeMap::from([("scrollbar.track".to_owned(), png(16))]),
    };
    (d, assets)
}

#[test]
fn an_imported_region_goes_through_the_validator() {
    let (d, assets) = with_region(Insets {
        top: 4,
        right: 4,
        bottom: 4,
        left: 4,
    });
    let canonical = validate::run(&package(&d, &assets).unwrap().bytes).unwrap();
    let m = crate::manifest::parse(&package::read(&canonical.bytes).unwrap()).unwrap();
    assert_eq!(
        m.regions["scrollbar.track"].image,
        "assets/regions/scrollbar.track.png"
    );
    let (bad, assets) = with_region(Insets {
        top: 9,
        right: 4,
        bottom: 9,
        left: 4,
    });
    let err = validate::run(&package(&bad, &assets).unwrap().bytes).unwrap_err();
    assert!(
        matches!(err.rule, SkinRule::NineSliceInsets { .. }),
        "{err:?}"
    );
    assert!(
        package(&bad, &no_assets()).is_err(),
        "a region without its bytes"
    );
}

/// The Linux walk (2026-11-18): "Start from the current skin" on
/// Symbiosis said "1 checks fail" with every pair passing. The one
/// failure was the draft's own empty author (`credits.authors[0] is
/// empty`), shown in the validator's words. Every built-in started as a
/// draft passes every pair; the only thing it asks for is its author,
/// named in the editor's words; with one it exports and validates.
#[test]
fn every_builtin_started_as_a_draft_passes_and_asks_only_for_its_author() {
    for pack in &crate::builtin::PACKS {
        let mut d = from_package(pack.id, pack.bytes).unwrap();
        let r = report(&d).unwrap();
        let bad: Vec<_> = r.pairs.iter().filter(|p| !p.pass).collect();
        assert!(bad.is_empty(), "builtin:{} {bad:?}", pack.id);
        // The missing author is the required field, not a failing check.
        assert!(
            r.failures.is_empty(),
            "builtin:{} {:?}",
            pack.id,
            r.failures
        );
        assert_eq!(r.author_required.as_deref(), Some(NO_AUTHOR));
        assert!(!r.exportable, "builtin:{} exports with no author", pack.id);
        d.author = "Bea".to_owned();
        let r = report(&d).unwrap();
        assert!(
            r.failures.is_empty(),
            "builtin:{} {:?}",
            pack.id,
            r.failures
        );
        assert_eq!(r.author_required, None, "builtin:{}", pack.id);
        assert!(r.exportable, "builtin:{}", pack.id);
        let assets = Assets {
            source: Some(pack.bytes.to_vec()),
            regions: BTreeMap::new(),
        };
        validate::run(&package(&d, &assets).unwrap().bytes)
            .unwrap_or_else(|e| panic!("builtin:{} {e}", pack.id));
    }
}

/// Plan E7.7.7 e777n: a fixed text on a tint over a derived ground (white
/// on `status.online 60% over bg.dark`, where `bg.dark` follows the skin's
/// `bg.primary`) is repaired by moving the colour the skin sets, never by
/// lowering the floor. Xfire Classic v3's palette: every ground #081020.
#[test]
fn the_walk_repairs_a_pair_over_a_derived_ground_through_the_skins_colour() {
    use crate::import::walk::{settle, WalkResult};
    let navy = color([0x08, 0x10, 0x20]);
    let mut tokens: BTreeMap<String, TokenValue> = [
        "bg.primary",
        "bg.secondary",
        "bg.panel",
        "bg.input",
        "bg.status",
    ]
    .into_iter()
    .map(|id| (id.to_owned(), navy.clone()))
    .collect();
    tokens.insert("accent.fill".to_owned(), color([0x1e, 0x49, 0x8c]));
    tokens.insert("accent.text".to_owned(), color([0x5c, 0x8c, 0xd5]));
    let build = |t: &BTreeMap<String, TokenValue>| {
        package(&draft(t.clone()), &no_assets())
            .map(|c| c.bytes)
            .map_err(crate::import::ImportError::Refused)
    };
    let (result, changes) = settle(&mut tokens, Base::Dark, build).unwrap();
    assert!(
        matches!(result, WalkResult::Passed(_)),
        "{result:?} after {changes:?}"
    );
    let over_dark = changes
        .iter()
        .find(|c| c.ground.ends_with("over bg.dark") && c.token == "bg.primary")
        .unwrap_or_else(|| panic!("no pair over bg.dark walked bg.primary: {changes:?}"));
    assert!(
        !tokens.contains_key("bg.dark"),
        "the walk set the derived ground"
    );
    assert!(over_dark.required_x100 >= 450, "{over_dark:?}");
}

/// Lead round 5 e: only the dialog walk may give up chroma. `#ff0000`
/// `danger.text` on a `#616161` window passes at no lighter lightness of its own chroma
/// (the end clamps to #ffad92, 3.4:1; white would pass, 6.2:1), so the base
/// walk stops unsettled with the pack's red intact rather than greying it.
#[test]
fn the_base_walk_never_reduces_chroma() {
    use crate::import::walk::{settle, WalkResult};
    let red = color([0xff, 0x00, 0x00]);
    let mut tokens: BTreeMap<String, TokenValue> = [
        "bg.primary",
        "bg.secondary",
        "bg.panel",
        "bg.input",
        "bg.status",
    ]
    .into_iter()
    .map(|id| (id.to_owned(), color([0x61, 0x61, 0x61])))
    .collect();
    tokens.insert("danger.text".to_owned(), red.clone());
    let build = |t: &BTreeMap<String, TokenValue>| {
        package(&draft(t.clone()), &no_assets())
            .map(|c| c.bytes)
            .map_err(crate::import::ImportError::Refused)
    };
    let (result, changes) = settle(&mut tokens, Base::Dark, build).unwrap();
    let WalkResult::Unsettled(u) = result else {
        panic!("red on #616161 settled: {changes:?}")
    };
    assert!(
        u.why.starts_with("no lightness of danger.text"),
        "{u:?} after {changes:?}"
    );
    assert_eq!(tokens.get("danger.text"), Some(&red), "{changes:?}");
}
