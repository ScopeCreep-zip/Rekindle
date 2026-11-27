use super::*;

#[test]
fn the_committed_model_parses_whole() {
    let text = include_str!("model.tsv");
    let rows = parse_model(text).unwrap();
    assert!(rows.len() > 100, "{}", rows.len());
    assert_eq!(model().len(), rows.len());
}

#[test]
fn a_ground_share_may_be_a_fractional_percentage_above_zero_up_to_100() {
    assert_eq!(
        parse_ground("--a 99.9% over --b").unwrap().tints,
        vec![("--a", "99.9%")]
    );
    assert!(parse_ground("--a 100% over --b").is_ok());
    for bad in [
        "--a 0% over --b",
        "--a 100.1% over --b",
        "--a -5% over --b",
        "--a NaN% over --b",
    ] {
        assert!(parse_ground(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_translucent_text_token_composites_and_a_translucent_ground_refuses() {
    let tokens: BTreeMap<String, String> = [
        ("--w".to_string(), "#ffffff80".to_string()),
        ("--k".to_string(), "#000000".to_string()),
    ]
    .into_iter()
    .collect();
    let pair = ModelPair {
        selector: ".a",
        state: "text",
        value: "var(--w)",
        ground: "--k",
        alpha: 1.0,
        graphic: false,
    };
    let m = pair.measure(&tokens).unwrap();
    assert!((m.fg[0] - 128.0 / 255.0).abs() < 1e-9);
    let on_glass = ModelPair {
        ground: "--w",
        ..pair
    };
    assert!(on_glass
        .measure(&tokens)
        .unwrap_err()
        .contains("translucent"));
}
