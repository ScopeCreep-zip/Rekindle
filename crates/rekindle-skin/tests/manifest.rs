//! Validator step 2: `manifest.json` and its text rules.

use rekindle_skin::limits::MAX_MANIFEST_BYTES;
use rekindle_skin::{manifest, package};
use rekindle_types::dto::skin::{
    SkinBase, SkinError, SkinLicense, SkinManifest, SkinRule, SKIN_FORMAT_PREFIX,
};
use serde_json::{json, Value};

use crate::common::{self, Entry};

fn parse_with(m: &Value, extra: &[Entry]) -> Result<SkinManifest, SkinError> {
    let bytes = common::package(m, extra);
    let p = package::read(&bytes).expect("the zip walk passes");
    manifest::parse(&p)
}

/// The minimal manifest with `key` set to `value`, refused.
fn refused_with(key: &str, value: Value) -> SkinError {
    let mut m = common::manifest();
    m[key] = value;
    parse_with(&m, &[]).expect_err("the manifest must be refused")
}

fn assert_rule(e: &SkinError, rule: &SkinRule) {
    assert_eq!((e.entry.as_str(), &e.rule), ("manifest.json", rule), "{e}");
}

fn field(name: &str) -> String {
    name.to_owned()
}

fn bad_value(e: &SkinError, name: &str) {
    match &e.rule {
        SkinRule::BadValue { field, .. } => assert_eq!(field, name, "{e}"),
        other => panic!("expected BadValue({name}), got {other:?}"),
    }
}

fn syntax(e: &SkinError) {
    assert!(matches!(e.rule, SkinRule::ManifestSyntax { .. }), "{e}");
}

#[test]
fn the_minimal_manifest_parses() {
    let m = parse_with(&common::manifest(), &[]).unwrap();
    assert_eq!(m.name, "Classic Xfire");
    assert_eq!(m.license, SkinLicense::Spdx("MIT".into()));
    assert_eq!(m.variants[0].base, SkinBase::Dark);
}

#[test]
fn every_optional_field_parses_and_round_trips() {
    let mut m = common::manifest();
    m["source"] = json!({ "format": "xfire-symbiosis", "hash": "ab".repeat(32) });
    m["author"] = json!("cd".repeat(32));
    m["signature"] = json!("ef".repeat(64));
    m["license"] = json!({ "text": "All rights reserved.\nRedistribution allowed." });
    m["description"] = json!("Line one.\nLine two.");
    m["name"] = json!("\u{3042}".repeat(64));
    m["variants"] = json!([
        { "base": "dark", "contrast": "standard", "tokens": "dark.json" },
        { "base": "light", "contrast": "high", "tokens": "light-hc.json" }
    ]);
    m["regions"] = json!({ "titlebar": {
        "image": "assets/frame.png",
        "fit": { "type": "nineSlice", "data": { "top": 4, "right": 4, "bottom": 4, "left": 4 } },
        "states": { "windowInactive": "assets/frame.png" },
        "textBacking": "titlebar-start"
    }});
    m["icons"] = json!({ "status.online": "icons/online.png" });
    m["sounds"] = json!({ "message": "sounds/in.wav" });
    let parsed = parse_with(&m, &[Entry::file("light-hc.json", b"{}")]).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), m);
}

#[test]
fn a_manifest_over_64_kib_is_refused() {
    let mut m = common::manifest();
    m["description"] = json!(" ".repeat(usize::try_from(MAX_MANIFEST_BYTES).unwrap()));
    let e = parse_with(&m, &[]).unwrap_err();
    assert!(
        matches!(
            e.rule,
            SkinRule::ManifestTooLarge {
                max: MAX_MANIFEST_BYTES,
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn malformed_json_is_refused() {
    let bytes = common::zip(&[
        Entry::file("manifest.json", b"{\"format\":"),
        Entry::file("dark.json", b"{}"),
    ]);
    syntax(&manifest::parse(&package::read(&bytes).unwrap()).unwrap_err());
}

#[test]
fn an_unknown_key_is_refused() {
    syntax(&refused_with("css", json!("body { background: url(x) }")));
}

#[test]
fn an_unknown_major_is_refused_before_any_other_key_is_read() {
    let m = json!({ "format": format!("{SKIN_FORMAT_PREFIX}2"), "futureKey": true });
    assert_rule(
        &parse_with(&m, &[]).unwrap_err(),
        &SkinRule::UnsupportedFormat { major: 2 },
    );
}

#[test]
fn a_malformed_format_is_refused() {
    // The domain-label gate keeps `rekindle-…/<n>` literals in the registry,
    // so the malformed majors are built on the prefix constant.
    for major in ["", "01", "1.0", "+1"] {
        let format = format!("{SKIN_FORMAT_PREFIX}{major}");
        bad_value(&refused_with("format", json!(format)), "format");
    }
    for format in ["other-skin/1", "rekindle-skin"] {
        bad_value(&refused_with("format", json!(format)), "format");
    }
}

#[test]
fn a_bad_id_is_refused() {
    for id in ["", "Classic", "-lead", "has space", &"a".repeat(65)] {
        bad_value(&refused_with("id", json!(id)), "id");
    }
}

#[test]
fn an_empty_name_is_refused() {
    assert_rule(
        &refused_with("name", json!("  ")),
        &SkinRule::EmptyText {
            field: field("name"),
        },
    );
}

#[test]
fn a_name_over_64_scalars_is_refused() {
    let rule = SkinRule::TextTooLong {
        field: field("name"),
        scalars: 65,
        max: 64,
    };
    assert_rule(&refused_with("name", json!("\u{3042}".repeat(65))), &rule);
}

#[test]
fn a_name_not_in_nfc_is_refused() {
    assert_rule(
        &refused_with("name", json!("Cafe\u{301}")),
        &SkinRule::NotNfc {
            field: field("name"),
        },
    );
}

#[test]
fn bidi_and_control_characters_in_the_name_are_refused() {
    for c in [
        '\u{202E}', '\u{2066}', '\u{200F}', '\u{61C}', '\n', '\u{0}', '\u{85}',
    ] {
        let rule = SkinRule::ForbiddenCharacter {
            field: field("name"),
            codepoint: u32::from(c),
        };
        assert_rule(
            &refused_with("name", json!(format!("Xfire{c}gpj.exe"))),
            &rule,
        );
    }
}

#[test]
fn a_description_over_280_scalars_is_refused() {
    let rule = SkinRule::TextTooLong {
        field: field("description"),
        scalars: 281,
        max: 280,
    };
    assert_rule(&refused_with("description", json!("d".repeat(281))), &rule);
}

#[test]
fn a_bidi_override_in_the_description_is_refused() {
    let rule = SkinRule::ForbiddenCharacter {
        field: field("description"),
        codepoint: 0x202E,
    };
    assert_rule(&refused_with("description", json!("ok\u{202E}txt")), &rule);
}

#[test]
fn a_bad_spdx_expression_is_refused() {
    for expr in ["MIT OR", "mit", "Not-A-License", "MIT/Apache-2.0"] {
        let e = refused_with("license", json!({ "spdx": expr }));
        assert!(matches!(e.rule, SkinRule::BadSpdx { .. }), "{expr}: {e}");
    }
}

#[test]
fn an_empty_license_text_is_refused() {
    let e = refused_with("license", json!({ "text": "" }));
    assert_rule(
        &e,
        &SkinRule::EmptyText {
            field: field("license"),
        },
    );
}

#[test]
fn a_bad_source_is_refused() {
    let e = refused_with(
        "source",
        json!({ "format": "xfire", "hash": "AB".repeat(32) }),
    );
    bad_value(&e, "source.hash");
    let e = refused_with(
        "source",
        json!({ "format": "X Fire", "hash": "ab".repeat(32) }),
    );
    bad_value(&e, "source.format");
}

fn credits(authors: Value, url: Value, licence: Value) -> Value {
    let mut c = serde_json::Map::new();
    c.insert("authors".into(), authors);
    c.insert("contributors".into(), json!([]));
    c.insert("url".into(), url);
    c.insert("licence".into(), licence);
    Value::Object(c)
}

#[test]
fn credits_parse_and_round_trip() {
    use rekindle_types::dto::skin::{OriginalLicence, SkinCredits};
    let mut m = common::manifest();
    m["credits"] = json!({
        "authors": ["Xfire, Inc."],
        "contributors": ["Tim Brengle"],
        "url": "http://www.xfire.com/",
        "licence": "noneStated",
    });
    let parsed = parse_with(&m, &[]).unwrap();
    let want = SkinCredits {
        authors: vec!["Xfire, Inc.".into()],
        contributors: vec!["Tim Brengle".into()],
        url: Some("http://www.xfire.com/".into()),
        licence: OriginalLicence::NoneStated,
    };
    assert_eq!(parsed.credits.as_ref(), Some(&want));
    m["credits"] = credits(
        json!([]),
        Value::Null,
        json!({ "text": "Free.\nShare it." }),
    );
    assert!(parse_with(&m, &[]).is_ok());
}

#[test]
fn bad_credits_are_refused() {
    let names = |n: usize| json!(vec!["A"; n]);
    let none = json!("noneStated");
    let e = refused_with("credits", credits(names(17), Value::Null, none.clone()));
    bad_value(&e, "credits.authors");
    let e = refused_with(
        "credits",
        credits(json!(["Al\u{202E}ice"]), Value::Null, none.clone()),
    );
    assert_rule(
        &e,
        &SkinRule::ForbiddenCharacter {
            field: field("credits.authors[0]"),
            codepoint: 0x202E,
        },
    );
    let e = refused_with(
        "credits",
        credits(json!(["A\nB"]), Value::Null, none.clone()),
    );
    assert!(matches!(e.rule, SkinRule::ForbiddenCharacter { .. }), "{e}");
    let e = refused_with("credits", credits(json!([" "]), Value::Null, none.clone()));
    assert!(matches!(e.rule, SkinRule::EmptyText { .. }), "{e}");
    let e = refused_with(
        "credits",
        credits(json!(["a".repeat(65)]), Value::Null, none.clone()),
    );
    assert!(matches!(e.rule, SkinRule::TextTooLong { .. }), "{e}");
    for url in [
        "javascript:alert(1)".to_owned(),
        "https://a b".to_owned(),
        "https://\u{0430}.example".to_owned(),
        format!("https://{}", "a".repeat(512)),
    ] {
        let e = refused_with("credits", credits(json!([]), json!(url), none.clone()));
        bad_value(&e, "credits.url");
    }
    let e = refused_with(
        "credits",
        credits(json!([]), Value::Null, json!({ "text": "x\u{2066}" })),
    );
    assert!(matches!(e.rule, SkinRule::ForbiddenCharacter { .. }), "{e}");
    let e = refused_with(
        "credits",
        json!({ "authors": [], "contributors": [], "url": null, "licence": "noneStated", "x": 1 }),
    );
    syntax(&e);
}

#[test]
fn a_bad_author_key_is_refused() {
    bad_value(&refused_with("author", json!("cd".repeat(31))), "author");
}

#[test]
fn a_signature_without_an_author_or_of_the_wrong_length_is_refused() {
    bad_value(
        &refused_with("signature", json!("ef".repeat(64))),
        "signature",
    );
    let mut m = common::manifest();
    m["author"] = json!("cd".repeat(32));
    m["signature"] = json!("ef".repeat(63));
    bad_value(&parse_with(&m, &[]).unwrap_err(), "signature");
}

#[test]
fn no_variants_are_refused() {
    assert_rule(&refused_with("variants", json!([])), &SkinRule::NoVariants);
}

#[test]
fn two_variants_with_one_base_and_contrast_are_refused() {
    let v = json!({ "base": "dark", "contrast": "standard", "tokens": "dark.json" });
    let rule = SkinRule::DuplicateVariant {
        base: "dark".into(),
        contrast: "standard".into(),
    };
    assert_rule(&refused_with("variants", json!([v, v])), &rule);
}

#[test]
fn a_variant_token_file_outside_the_root_or_the_manifest_is_refused() {
    for tokens in ["assets/dark.json", "manifest.json", "dark.css"] {
        let v = json!([{ "base": "dark", "contrast": "standard", "tokens": tokens }]);
        bad_value(&refused_with("variants", v), "variants[0].tokens");
    }
}

#[test]
fn a_variant_token_file_the_package_lacks_is_refused() {
    let v = json!([{ "base": "light", "contrast": "standard", "tokens": "light.json" }]);
    let e = refused_with("variants", v);
    let rule = SkinRule::MissingEntry {
        field: field("variants[0].tokens"),
    };
    assert_eq!((e.entry.as_str(), &e.rule), ("light.json", &rule), "{e}");
}

#[test]
fn an_unknown_variant_key_is_refused() {
    let v = json!([{ "base": "dark", "contrast": "standard", "tokens": "dark.json", "css": "x" }]);
    syntax(&refused_with("variants", v));
}

#[test]
fn an_unknown_icon_key_is_refused() {
    let mut m = common::manifest();
    m["icons"] = json!({ "status.available": "icons/a.png" });
    let e = parse_with(&m, &[]).unwrap_err();
    assert!(
        matches!(&e.rule, SkinRule::UnknownIcon { icon } if icon == "status.available"),
        "{e}"
    );
}

#[test]
fn an_unknown_sound_key_is_refused() {
    let mut m = common::manifest();
    m["sounds"] = json!({ "message.received": "sounds/in.wav" });
    let e = parse_with(&m, &[]).unwrap_err();
    assert!(
        matches!(&e.rule, SkinRule::UnknownSoundCue { cue } if cue == "message.received"),
        "{e}"
    );
}

#[test]
fn every_registry_icon_slot_and_sound_cue_is_a_key() {
    use rekindle_skin::registry::{IconSlot, SoundCue};
    let mut m = common::manifest();
    let icons: serde_json::Map<_, _> = IconSlot::ALL
        .iter()
        .map(|s| (s.id().to_owned(), json!(format!("icons/{}.png", s.id()))))
        .collect();
    let sounds: serde_json::Map<_, _> = SoundCue::ALL
        .iter()
        .map(|c| (c.id().to_owned(), json!(format!("sounds/{}.wav", c.id()))))
        .collect();
    m["icons"] = icons.into();
    m["sounds"] = sounds.into();
    parse_with(&m, &[]).unwrap();
    for c in SoundCue::ALL {
        assert_eq!(serde_json::to_value(c).unwrap(), json!(c.id()));
    }
}
