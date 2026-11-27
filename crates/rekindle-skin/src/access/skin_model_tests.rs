//! The pair model on resolved variants ([`super`]).

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinRule, TokenValue};

use super::{model_failures, model_pairs, pairs_on};
use crate::access::failures;
use crate::access::model::ModelPair;
use crate::registry::{dialog, entry_in, resolve, Base, BASE_TABLE, ENTRIES};

/// The custom property of registry row `id`.
fn css(id: &str) -> &'static str {
    entry_in(ENTRIES, id).map_or("", |e| e.css)
}

/// A text pair of `value` on `ground`, named `.probe`.
fn probe<'a>(value: &'a str, ground: &'a str) -> ModelPair<'a> {
    ModelPair {
        selector: ".probe",
        state: "text",
        value,
        ground,
        alpha: 1.0,
        graphic: false,
    }
}

/// Every pair of the model resolves on every base's registry defaults:
/// it names only registry tokens and opaque grounds.
#[test]
fn every_model_pair_resolves_on_the_registry_defaults() {
    for base in Base::ALL {
        let resolved = resolve(ENTRIES, base, &BTreeMap::new());
        let pairs = model_pairs(&resolved);
        assert!(pairs.len() > 100);
        let bad: Vec<_> = pairs.iter().filter_map(|p| p.as_ref().err()).collect();
        assert!(bad.is_empty(), "{}: {bad:?}", base.key());
    }
}

fn is_model(rule: &SkinRule) -> bool {
    matches!(rule, SkinRule::Contrast { rule, .. } if rule.contains(" ."))
}

/// A skin whose window ground is #161616 keeps the default accent
/// text, which passes on every opaque ground but reads 4.44:1 on the
/// accent tint `.search-panel-chip-active` is drawn on: the registry
/// constraints and `PAIRS` pass it, the full model refuses it.
#[test]
fn the_validator_refuses_a_skin_whose_tint_pair_fails() {
    let base = Base::Dark;
    let ground = TokenValue::Color {
        r: 22,
        g: 22,
        b: 22,
        a: 255,
    };
    // The window ground stays the default: it follows `bg.primary`
    // unless the skin sets it (plan E7.7.7 e777n), and this test is
    // about the chip's tint over `bg.primary` alone.
    let window = TokenValue::Color {
        r: 10,
        g: 10,
        b: 10,
        a: 255,
    };
    let tokens: BTreeMap<String, TokenValue> = [
        ("bg.primary".to_owned(), ground),
        ("bg.dark".to_owned(), window),
    ]
    .into_iter()
    .collect();
    let resolved = resolve(ENTRIES, base, &tokens);
    let all = failures(base, &resolved);
    assert!(!all.is_empty(), "the tint pair passes");
    assert!(all.iter().all(is_model), "{all:?}");
    assert_eq!(all, model_failures(&resolved));
    assert!(
        all.iter().any(|r| matches!(
            r,
            SkinRule::Contrast { token, ground, .. }
                if token == "accent.text" && ground.contains(" over ")
        )),
        "{all:?}"
    );
}

/// A pair that reads a token the sheet lacks is refused with the measure's
/// own reason, never as a translucent ground. Fail-without: mapping every
/// measure error to `TranslucentGround` (the old `map_err`) refuses it as
/// `TranslucentGround { token: "bg.panel" }`.
#[test]
fn a_pair_reading_an_unknown_token_names_its_real_reason() {
    let resolved = resolve(ENTRIES, Base::Dark, &BTreeMap::new());
    let value = "var(--color-xfire-no-such-token)";
    let got = pairs_on(&[probe(value, css("bg.panel"))], &resolved);
    assert_eq!(
        got,
        vec![Err(SkinRule::UnmeasuredPair {
            rule: "1.4.3 .probe".to_owned(),
            why: format!("`{value}` names no `@theme` colour token"),
        })]
    );
}

/// A pair whose opaque ground the skin draws translucent is refused as that
/// translucent ground, named by its registry id.
#[test]
fn a_pair_on_a_translucent_ground_names_the_ground() {
    let panel = TokenValue::Color {
        r: 30,
        g: 30,
        b: 30,
        a: 0x80,
    };
    let tokens: BTreeMap<String, TokenValue> =
        [("bg.panel".to_owned(), panel)].into_iter().collect();
    let resolved = resolve(ENTRIES, Base::Dark, &tokens);
    let value = format!("var({})", css("text.primary"));
    let got = pairs_on(&[probe(&value, css("bg.panel"))], &resolved);
    assert_eq!(
        got,
        vec![Err(SkinRule::TranslucentGround {
            token: "bg.panel".to_owned(),
        })]
    );
}

/// Over the base walk's table, which holds the dialog roles out (ST1 ruling
/// 2), a pair reading a dialog role is not a pair of that table, while a
/// base pair is measured; over every row the role is served and the pair
/// measured. Fail-without: dropping the `holds_out` filter refuses the role
/// pair over `BASE_TABLE` as `UnmeasuredPair` (what failed every import's
/// base walk once the model carried the dialog pairs).
#[test]
fn a_pair_reading_a_held_out_dialog_role_is_not_a_pair_of_the_base_table() {
    assert!(dialog::is_role("selection.text"));
    assert!(entry_in(BASE_TABLE, "selection.text").is_none());
    let role = format!("var({})", css("selection.text"));
    let text = format!("var({})", css("text.primary"));
    let pairs = [probe(&role, css("bg.panel")), probe(&text, css("bg.panel"))];
    let base_rows = resolve(BASE_TABLE, Base::Dark, &BTreeMap::new());
    let got = pairs_on(&pairs, &base_rows);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(
        matches!(&got[0], Ok(m) if m.fg == "text.primary"),
        "{got:?}"
    );
    let every_row = resolve(ENTRIES, Base::Dark, &BTreeMap::new());
    let got = pairs_on(&pairs, &every_row);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        matches!(&got[0], Ok(m) if m.fg == "selection.text"),
        "{got:?}"
    );
}
