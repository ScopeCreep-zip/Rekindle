use super::*;

fn tokens() -> BTreeMap<String, String> {
    grounds::theme_colours(
        "@theme {\n  --color-w: #ffffff;\n  --color-k: #000000;\n  --color-p: #1e1e1e;\n}",
    )
}

const MAP_TEXT: &str =
    "[selectors]\n'.a' = { on = [\"--color-k\"] }\n[sec]\n[boundaries]\n[dimmers]\n";

fn sheet(css: &str) -> Vec<(String, String)> {
    vec![("src/styles/theme/00-x.css".into(), css.into())]
}

fn problems(css: &str, map_text: &str) -> Vec<String> {
    let map = grounds::read_map(map_text).unwrap();
    let model = read_model(map_text).unwrap();
    structure(&sheet(css), &map, &model, &tokens(), &BTreeMap::new())
}

#[test]
fn paints_resolve() {
    let t = tokens();
    let white = |p: Option<Paint>| matches!(p, Some(Paint::Opaque(c)) if close(c, [1.0; 3]));
    assert!(white(paint(&t, "var(--color-w)").unwrap()));
    assert!(matches!(
        paint(&t, "color-mix(in srgb, var(--color-w) 30%, transparent)").unwrap(),
        Some(Paint::Tint(_, p)) if (p - 30.0).abs() < 0.01
    ));
    let half = paint(&t, "color-mix(in srgb, var(--color-w), var(--color-k) 50%)").unwrap();
    assert!(matches!(half, Some(Paint::Opaque(c)) if close(c, [0.5; 3])));
    assert!(paint(&t, "transparent").unwrap().is_none());
    let stops = paint(
        &t,
        "linear-gradient(to right, var(--color-w), var(--color-k) 60%)",
    );
    assert!(matches!(stops, Ok(Some(Paint::Stops(s))) if s.len() == 2));
    assert!(paint(&t, "radial-gradient(var(--color-w), var(--color-k))").is_err());
}

#[test]
fn an_own_ground_must_be_declared() {
    let css = ".a { color: var(--color-w); background: var(--color-p); }";
    let found = problems(css, MAP_TEXT);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("on `--color-k` is not the background"));
    let ok = MAP_TEXT.replace("\"--color-k\"", "\"--color-p\"");
    assert!(problems(css, &ok).is_empty());
}

#[test]
fn a_hover_ground_must_be_declared_in_on_hover() {
    let css = ".a { color: var(--color-w); }\n.a:hover { background: color-mix(in srgb, var(--color-w) 10%, transparent); }";
    let found = problems(css, MAP_TEXT);
    assert!(found[0].contains("list it in `on_hover`"), "{found:?}");
    let ok = MAP_TEXT.replace("] }", "], on_hover = [\"--color-w 10% over --color-k\"] }");
    assert!(problems(css, &ok).is_empty());
}

#[test]
fn brightness_opacity_rings_and_sec_need_their_tables() {
    let css = ".a { color: var(--color-w); }\n.b:hover { filter: brightness(1.2); }\n.row { opacity: 0.5; }\n.x:disabled { opacity: 0.4; }\n*:focus-visible { outline: 2px solid var(--color-w); }\n@layer sec { .s { color: var(--sec-fg); } }";
    let found = problems(css, MAP_TEXT);
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(found.iter().any(|p| p.contains("brightness(1.2)")));
    assert!(found.iter().any(|p| p.contains("`.row` sets an opacity")));
    assert!(found
        .iter()
        .any(|p| p.contains("`*:focus-visible` draws a focus ring")));
    assert!(found
        .iter()
        .any(|p| p.contains("`.s` colours `@layer sec` text")));
}

#[test]
fn opacities_multiply_onto_the_dimmed_text() {
    let css = ".a { color: var(--color-w); opacity: 0.5; }\n.row { opacity: 0.8; }\n.a:disabled { opacity: 0.1; }";
    let text = MAP_TEXT.replace("[dimmers]\n", "[dimmers]\n'.row' = { dims = [\".a\"] }\n");
    let map = grounds::read_map(&text).unwrap();
    let alpha = alphas(&sheet(css), &map, &read_model(&text).unwrap(), false);
    assert!((alpha[".a"] - 0.4).abs() < 1e-9);
}

#[test]
fn a_shorthand_yields_its_colour() {
    assert_eq!(
        shorthand_colour("1px solid color-mix(in srgb, var(--a) 20%, transparent)"),
        "color-mix(in srgb, var(--a) 20%, transparent)"
    );
    assert_eq!(shorthand_colour("2px solid var(--a)"), "var(--a)");
}

#[test]
fn sec_bases_overlay_in_order() {
    let global = "@layer sec {\n  :root { --sec-fg: #ffffff; --sec-bg: #000000; }\n  :root[data-skin-base=\"light\"] { --sec-fg: #111111; }\n  @media (prefers-contrast: more) {\n    :root, :root[data-skin-base=\"light\"] { --sec-bg: #222222; }\n  }\n}";
    let bases: BTreeMap<_, _> = sec_bases(global).into_iter().collect();
    assert_eq!(bases["dark"]["--sec-fg"], "#ffffff");
    assert_eq!(bases["light"]["--sec-fg"], "#111111");
    assert_eq!(bases["dark+hc"]["--sec-bg"], "#222222");
    assert_eq!(bases["light+hc"]["--sec-fg"], "#111111");
}
