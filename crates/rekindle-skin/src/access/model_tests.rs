use super::*;

#[test]
fn the_committed_model_parses_whole() {
    let text = include_str!("model.tsv");
    let rows = parse_model(text).unwrap();
    assert!(rows.len() > 100, "{}", rows.len());
    assert_eq!(model().len(), rows.len());
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
