//! Validator step 3: the token profile, the re-emitter and the registry
//! defaults (plan E7.7.1).

use rekindle_skin::registry::{
    self, constraint_failures, resolve, Base, Class, DefaultValue, Defaults, TokenEntry, TokenType,
    ENTRIES,
};
use rekindle_skin::tokens::emit::{css_value, in_alphabet};
use rekindle_skin::tokens::{parse, parse_with};
use rekindle_types::dto::skin::{SkinRule, TokenValue};
use serde_json::{json, Value};

const NUM: TokenEntry = TokenEntry {
    id: "test.ratio",
    css: "--test-ratio",
    ty: TokenType::Number { min: 0, max: 2000 },
    defaults: Defaults {
        dark: DefaultValue::Number(500),
        light: DefaultValue::Number(500),
        hc_dark: DefaultValue::Number(500),
        hc_light: DefaultValue::Number(500),
    },
    derivation: None,
    constraints: &[],
    class: Class::Themeable,
};

fn rule(v: &Value) -> SkinRule {
    parse("dark.json", v.to_string().as_bytes())
        .expect_err("refused")
        .rule
}

fn ok(v: &Value) -> std::collections::BTreeMap<String, TokenValue> {
    parse("dark.json", v.to_string().as_bytes()).expect("accepted")
}

#[test]
fn a_valid_file_parses_with_groups_aliases_and_both_colour_forms() {
    let t = ok(&json!({
        "bg": { "$type": "color",
                "dark": { "$value": "#102030" },
                "panel": { "$value": { "colorSpace": "srgb", "components": [1, 0, 0], "alpha": 0.5 } },
                "primary": { "$value": "{bg.dark}" } },
        "size": { "chrome-height": { "$type": "dimension", "$value": { "value": 30, "unit": "px" } } },
        "font": { "family": { "$type": "fontFamily", "$value": "Tahoma" } }
    }));
    assert_eq!(
        t["bg.primary"],
        TokenValue::Color {
            r: 0x10,
            g: 0x20,
            b: 0x30,
            a: 255
        }
    );
    assert_eq!(
        t["bg.panel"],
        TokenValue::Color {
            r: 255,
            g: 0,
            b: 0,
            a: 128
        }
    );
    assert_eq!(t["size.chrome-height"], TokenValue::Dimension(30));
}

#[test]
fn each_profile_rule_refuses_the_file() {
    let c = |v: Value| json!({ "bg": { "dark": v } });
    assert!(matches!(
        rule(&json!({ "nope": { "$type": "color", "$value": "#000000" } })),
        SkinRule::UnknownToken { .. }
    ));
    assert!(matches!(
        rule(&json!({ "shadow": { "$type": "color", "$value": "#000000" } })),
        SkinRule::FixedToken { .. }
    ));
    assert!(matches!(
        rule(&c(
            json!({ "$type": "color", "$value": "#000000", "$ref": "http://x" })
        )),
        SkinRule::TokenKey { .. }
    ));
    assert!(matches!(
        rule(&c(
            json!({ "$type": "color", "$value": "#000000", "$extensions": { "com.other": {} } })
        )),
        SkinRule::TokenKey { .. }
    ));
    assert!(matches!(
        rule(&c(json!({ "$type": "dimension", "$value": "#000000" }))),
        SkinRule::TokenTypeMismatch { .. }
    ));
    for bad in [
        "url(x)",
        "var(--a)",
        "calc(1px)",
        "#00000",
        "red",
        "#0000zz",
    ] {
        assert!(
            matches!(
                rule(&c(json!({ "$type": "color", "$value": bad }))),
                SkinRule::BadTokenValue { .. }
            ),
            "{bad}"
        );
    }
    let dim =
        |v: Value| json!({ "size": { "chrome-height": { "$type": "dimension", "$value": v } } });
    for bad in [
        json!({ "value": 41, "unit": "px" }),
        json!({ "value": 30, "unit": "em" }),
        json!({ "value": 30.5, "unit": "px" }),
        json!("30px"),
    ] {
        assert!(
            matches!(rule(&dim(bad.clone())), SkinRule::BadTokenValue { .. }),
            "{bad}"
        );
    }
    assert!(matches!(
        rule(&json!({ "font": { "family": { "$type": "fontFamily", "$value": "Comic Evil" } } })),
        SkinRule::BadTokenValue { .. }
    ));
    assert!(matches!(
        rule(&json!({ "Bg": { "dark": { "$type": "color", "$value": "#000000" } } })),
        SkinRule::TokenKey { .. }
    ));
}

#[test]
fn alias_cycles_and_dangling_aliases_are_refused() {
    let cyc = json!({ "bg": { "$type": "color", "dark": { "$value": "{bg.panel}" }, "panel": { "$value": "{bg.dark}" } } });
    assert!(matches!(rule(&cyc), SkinRule::AliasCycle { .. }));
    let dangling = json!({ "bg": { "$type": "color", "dark": { "$value": "{bg.nowhere}" } } });
    assert!(matches!(rule(&dangling), SkinRule::UnresolvedAlias { .. }));
}

#[test]
fn numbers_take_at_most_three_decimals_and_their_range() {
    let p = |v: Value| {
        parse_with(
            &[NUM],
            "t.json",
            json!({ "test": { "ratio": { "$type": "number", "$value": v } } })
                .to_string()
                .as_bytes(),
        )
    };
    assert_eq!(
        p(json!(1.25)).expect("ok")["test.ratio"],
        TokenValue::Number(1250)
    );
    // A fourth decimal is refused even when the value is in range.
    assert!(p(json!(0.0005)).is_err());
    assert!(p(json!(1.2345)).is_err());
    assert!(p(json!(2.001)).is_err());
    assert!(p(json!(-0.5)).is_err());
}

#[test]
fn the_re_emitter_alphabet_never_carries_skin_text() {
    let hostile = "#000000;}body{background:url(https://evil/x)}";
    assert!(parse(
        "dark.json",
        json!({ "bg": { "dark": { "$type": "color", "$value": hostile } } })
            .to_string()
            .as_bytes()
    )
    .is_err());
    let t = ok(&json!({ "bg": { "dark": { "$type": "color", "$value": "#AbCdEf80" } } }));
    for base in Base::ALL {
        for (e, v) in resolve(ENTRIES, base, &t) {
            let css = css_value(e, &v).expect("fits");
            if e.ty == TokenType::FontFamily {
                assert!(registry::FONTS.iter().any(|f| f.stack == css));
            } else {
                assert!(in_alphabet(&css), "{} = {css}", e.id);
                assert!(!css.contains("url") && !css.contains('}'));
            }
        }
    }
    let e = registry::entry_in(ENTRIES, "bg.dark").expect("row");
    assert_eq!(css_value(e, &t["bg.dark"]).as_deref(), Some("#abcdef80"));
}

#[test]
fn registry_defaults_pass_their_constraints_on_every_base() {
    for base in Base::ALL {
        assert_eq!(
            constraint_failures(ENTRIES, base),
            Vec::<String>::new(),
            "{base:?}"
        );
    }
    for e in ENTRIES {
        assert!(
            !ENTRIES
                .iter()
                .any(|o| o.id.starts_with(&format!("{}.", e.id))),
            "{} is a token and a group",
            e.id
        );
        for base in Base::ALL {
            assert!(
                css_value(e, &e.defaults.on(base).value()).is_some(),
                "{} {base:?}",
                e.id
            );
        }
        if let Some(d) = e.derivation {
            for s in d.sources() {
                assert!(
                    registry::entry_in(ENTRIES, s).is_some_and(|r| r.derivation.is_none()),
                    "{}",
                    e.id
                );
            }
        }
    }
}

/// Plan E7.7.7: window, menu and panel edges (`border`) follow a skin that
/// sets only its accent, and keep the default when no skin is applied.
#[test]
fn a_skins_accent_tints_the_edges() {
    let border = |skin: &std::collections::BTreeMap<String, TokenValue>| {
        resolve(ENTRIES, Base::Dark, skin)
            .into_iter()
            .find(|(e, _)| e.id == "border")
            .map(|(_, v)| v)
            .expect("border is a registry token")
    };
    let plain = border(&std::collections::BTreeMap::new());
    assert_eq!(
        plain,
        TokenValue::Color {
            r: 0x2a,
            g: 0x2a,
            b: 0x2a,
            a: 255
        }
    );
    let skinned = border(&ok(
        &json!({ "accent": { "fill": { "$type": "color", "$value": "#ff0000" } } }),
    ));
    let TokenValue::Color { r, g, b, .. } = skinned else {
        panic!("border is a colour: {skinned:?}");
    };
    assert!(r > g && r > b, "the edge leans to the accent: {r} {g} {b}");
}
