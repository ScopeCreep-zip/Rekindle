use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinBase, SkinContrast, SkinRule};
use rekindle_types::dto::skin_draft::SkinDraft;

use super::*;
use crate::validate;

fn draft(tokens: BTreeMap<String, TokenValue>) -> SkinDraft {
    SkinDraft {
        name: "My Skin".to_owned(),
        base: SkinBase::Dark,
        contrast: SkinContrast::Standard,
        tokens,
    }
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
    let built = package(&failing()).unwrap();
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
    let built = package(&d).unwrap();
    let canonical = validate::run(&built.bytes).unwrap();
    assert_eq!(
        canonical.bytes, built.bytes,
        "the export is already canonical"
    );
    assert_eq!(
        validate::run(&canonical.bytes).unwrap().bytes,
        canonical.bytes
    );
    let back = from_package(&canonical.bytes).unwrap();
    assert_eq!(back, d);
    d.name = "  ".to_owned();
    assert_eq!(manifest_of(&d).id, "draft");
    assert_eq!(slug("Neon -- Night 2"), "neon-night-2");
}

#[test]
fn an_unknown_or_fixed_token_is_refused() {
    let unknown = draft(BTreeMap::from([("no.such".to_owned(), color([1, 2, 3]))]));
    assert!(report(&unknown).is_err());
    let fixed = draft(BTreeMap::from([("accent.fg".to_owned(), color([1, 2, 3]))]));
    assert!(report(&fixed).is_err());
}
