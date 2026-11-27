use super::*;

fn sheets() -> Vec<(String, String)> {
    let css = "@layer components {\n  .a, .b:hover { color: var(--color-x-text); }\n  .c { color: inherit; background: var(--color-x-bg); }\n  @media (x) { .d::placeholder { color: var(--color-x-dim, var(--color-x-text)); } }\n}\n@layer sec { .s { color: var(--sec-fg); } }\n@keyframes k { 0% { color: var(--color-x-text); } }";
    vec![("src/styles/theme/00-x.css".into(), css.into())]
}

fn tokens() -> BTreeSet<String> {
    theme_tokens("@theme {\n  --color-x-text: #fff;\n  --color-x-bg: #000;\n  --color-x-tint: #00f;\n  --font-x: a;\n}")
}

fn map(entries: &[(&str, &[&str])]) -> BTreeMap<String, Entry> {
    entries
        .iter()
        .map(|(s, on)| {
            (
                (*s).to_string(),
                Entry {
                    on: on.iter().map(|g| (*g).to_string()).collect(),
                    ..Entry::default()
                },
            )
        })
        .collect()
}

#[test]
fn finds_the_text_token_selectors() {
    let wanted = text_token_selectors(&sheets());
    assert_eq!(
        wanted.keys().collect::<Vec<_>>(),
        vec![".a", ".b:hover", ".d::placeholder"]
    );
    assert_eq!(wanted[".a"], "src/styles/theme/00-x.css:2");
    let rules = text_rules(&sheets());
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[2].value, "var(--color-x-dim, var(--color-x-text))");
}

#[test]
fn a_complete_true_map_passes() {
    let m = map(&[
        (".a", &["--color-x-bg"]),
        (".b:hover", &["--color-x-tint 30% over --color-x-bg"]),
        (
            ".d::placeholder",
            &[
                "--color-x-bg",
                "--color-x-tint 10% over --color-x-tint 5% over --color-x-bg",
            ],
        ),
    ]);
    let problems = compare(&text_token_selectors(&sheets()), &m, &tokens());
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_missing_empty_stale_or_malformed_entry_fails() {
    let wanted = text_token_selectors(&sheets());
    let missing = map(&[(".a", &["--color-x-bg"]), (".b:hover", &["--color-x-bg"])]);
    let p = compare(&wanted, &missing, &tokens());
    assert_eq!(p.len(), 1);
    assert!(p[0].contains("`.d::placeholder` colours text"), "{p:?}");

    let full = |extra: (&'static str, &'static [&'static str])| {
        let mut m = map(&[
            (".a", &["--color-x-bg"]),
            (".b:hover", &["--color-x-bg"]),
            (".d::placeholder", &["--color-x-bg"]),
        ]);
        m.extend(map(&[extra]));
        compare(&wanted, &m, &tokens())
    };
    assert!(full((".a", &[]))[0].contains("empty `on`"));
    assert!(full((".gone", &["--color-x-bg"]))[0].contains("is not a selector"));
    assert!(full((".a", &["--color-x-nope"]))[0].contains("not a `@theme` colour token"));
    assert!(full((".a", &["--font-x"]))[0].contains("not a `@theme` colour token"));
    assert!(full((".a", &["--color-x-nope 5% over --color-x-bg"]))[0]
        .contains("`--color-x-nope` is not"));
    assert!(full((".a", &["--color-x-tint 0% over --color-x-bg"]))[0].contains("percentage"));
    assert!(full((".a", &["--color-x-tint over --color-x-bg"]))[0].contains("<token> <N>%"));
}

#[test]
fn a_ground_parses_top_down() {
    let g = parse_ground("--a 10% over --b 5% over --c").unwrap();
    assert_eq!(g.tints, vec![("--a", "10%"), ("--b", "5%")]);
    assert_eq!(g.base, "--c");
    assert!(parse_ground("--a 101% over --c").is_err());
    let g = parse_ground("--a var(--tint-fill) over --c").unwrap();
    assert_eq!(g.tints, vec![("--a", "var(--tint-fill)")]);
    assert!(parse_ground("--a var(--color-a) over --c").is_err());
}

#[test]
fn the_map_format_reads() {
    let m = read_map("[selectors]\n'.a' = { on = [\"--color-x-bg\"] }\n'.b[data-x=\"y\"]' = { on = [\"--color-x-bg\", \"--color-x-tint 30% over --color-x-bg\"], graphic = true }\n").unwrap();
    assert_eq!(m[".b[data-x=\"y\"]"].on.len(), 2);
    assert!(m[".b[data-x=\"y\"]"].graphic && !m[".a"].graphic);
    assert!(read_map("[selectors]\n'.a' = { region = \"x\" }\n").is_err());
    assert!(read_map("[selectors]\n'.a' = { on = [], graphic = \"yes\" }\n").is_err());
}
