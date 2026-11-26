use super::*;
use rekindle_skin::access::model::{contrast, text_colour};

fn tokens() -> BTreeMap<String, String> {
    grounds::theme_colours(
        "@theme {\n  --color-w: #ffffff;\n  --color-k: #000;\n  --color-grey: #767676;\n  --color-red: #ef4444;\n  --color-panel: #1e1e1e;\n  --color-bad: rgb(1 2 3);\n}",
    )
}

fn rules(css: &str) -> Vec<TextRule> {
    grounds::text_rules(&[("src/styles/theme/00-x.css".into(), css.into())])
}

fn entry(on: &[&str], graphic: bool) -> Entry {
    Entry {
        on: on.iter().map(|g| (*g).to_string()).collect(),
        graphic,
        ..Entry::default()
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.005
}

#[test]
fn ratios_match_the_wcag_vectors() {
    let t = tokens();
    assert!(approx(
        contrast(
            token(&t, "--color-w").unwrap(),
            token(&t, "--color-k").unwrap()
        ),
        21.0
    ));
    assert!(approx(
        contrast(
            token(&t, "--color-grey").unwrap(),
            token(&t, "--color-w").unwrap()
        ),
        4.54
    ));
    assert!(approx(
        contrast(
            token(&t, "--color-red").unwrap(),
            token(&t, "--color-panel").unwrap()
        ),
        4.43
    ));
    assert!(token(&t, "--color-bad")
        .unwrap_err()
        .contains("not a hex colour"));
}

#[test]
fn grounds_composite_top_down() {
    let t = tokens();
    let half = ground(&t, "--color-w 50% over --color-k").unwrap();
    assert!(half.iter().all(|c| approx(*c, 0.5)));
    // 50% white over (50% white over black) = 75% white.
    let three_quarters = ground(&t, "--color-w 50% over --color-w 50% over --color-k").unwrap();
    assert!(three_quarters.iter().all(|c| approx(*c, 0.75)));
    assert!(ground(&t, "--color-nope").is_err());
}

#[test]
fn text_values_resolve() {
    let t = tokens();
    let k = token(&t, "--color-k").unwrap();
    let white = |rgb: Rgb| rgb.iter().all(|c| approx(*c, 1.0));
    assert!(white(
        text_colour(&t, "var(--color-w) !important", k).unwrap()
    ));
    assert!(white(
        text_colour(&t, "var(--profile-accent, var(--color-w))", k).unwrap()
    ));
    let faded = text_colour(&t, "color-mix(in srgb, var(--color-w) 60%, transparent)", k).unwrap();
    assert!(faded.iter().all(|c| approx(*c, 0.6)));
    assert!(text_colour(&t, "color-mix(in srgb, var(--color-w) 60%, black)", k).is_err());
    assert!(text_colour(&t, "var(--profile-accent)", k).is_err());
}

#[test]
fn a_pair_under_its_floor_fails_and_a_graphic_needs_three() {
    let css = "@layer components {\n  .ok { color: var(--color-w); }\n  .low { color: var(--color-red); }\n  .dot { color: var(--color-red); }\n}";
    let map: BTreeMap<String, Entry> = [
        (
            ".ok".to_string(),
            entry(&["--color-k", "--color-red 30% over --color-k"], false),
        ),
        (
            ".low".to_string(),
            entry(&["--color-k", "--color-panel"], false),
        ),
        (".dot".to_string(), entry(&["--color-panel"], true)),
    ]
    .into_iter()
    .collect();
    let report = measure_rows(
        &rows(rules(css).iter(), &map, &BTreeMap::new(), "text"),
        &tokens(),
    );
    assert_eq!(report.measured, 5);
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(report.problems[0].contains("`.low`"));
    assert!(report.problems[0].contains("on `--color-panel` reads 4.43:1, needs 4.5:1"));
}

#[test]
fn an_unresolvable_ground_fails() {
    let css = ".a { color: var(--color-w); }";
    let map = [(".a".to_string(), entry(&["--color-bad"], false))]
        .into_iter()
        .collect();
    let report = measure_rows(
        &rows(rules(css).iter(), &map, &BTreeMap::new(), "text"),
        &tokens(),
    );
    assert!(
        report.problems[0].contains("not a hex colour"),
        "{:?}",
        report.problems
    );
}

#[test]
fn the_light_pass_measures_every_rule_on_the_light_grounds() {
    let global = "@theme {\n  --color-xfire-bg-panel: #1e1e1e;\n  --color-xfire-away: #ffcc00;\n  --color-xfire-away-text: #ffcc00;\n  --color-xfire-text: #e7e7e7;\n}\n@layer xfire-defaults {\n  :root[data-skin-base=\"light\"] {\n    --color-xfire-bg-panel: #ffffff;\n    --color-xfire-away: #b59000;\n    --color-xfire-away-text: #8a6d00;\n  }\n}";
    let defaults = grounds::theme_colours(global);
    let light = grounds::light_base_colours(global);
    assert_eq!(light.len(), 3);
    let tokens = light_tokens(&defaults, &light);
    assert_eq!(tokens["--color-xfire-bg-panel"], "#ffffff");
    assert_eq!(tokens["--color-xfire-away"], "#b59000");
    let css = ".warn { color: var(--color-xfire-away-text); }\n.dot { color: var(--color-xfire-away); }\n.body { color: var(--color-xfire-text); }";
    let all = rules(css);
    let on = &[
        "--color-xfire-bg-panel",
        "--color-xfire-away 20% over --color-xfire-bg-panel",
    ];
    let map: BTreeMap<String, Entry> = [
        (".warn".to_string(), entry(on, false)),
        (".dot".to_string(), entry(&["--color-xfire-bg-panel"], true)),
    ]
    .into_iter()
    .collect();
    let report = measure_rows(&rows(all.iter(), &map, &BTreeMap::new(), "text"), &tokens);
    assert_eq!(report.measured, 3);
    // #8a6d00 on white is 4.96:1, on the light away tint under 4.5:1.
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(
        report.problems[0].contains("20% over"),
        "{:?}",
        report.problems
    );
}

/// The Verify of plan E7.7.0a: the shipped map and palette pass.
#[test]
fn the_default_palette_passes_every_declared_pair() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the workspace root");
    check_contrast_pairs(root).unwrap();
}
