use super::*;

#[test]
fn finds_hex_and_colour_functions() {
    assert_eq!(literals_in("1px solid #fff"), vec!["#fff"]);
    assert_eq!(
        literals_in("linear-gradient(to right, #0f1a3a, #1a2a5eCC)"),
        vec!["#0f1a3a", "#1a2a5eCC"]
    );
    assert_eq!(literals_in("0 1px rgb(0 0 0 / 0.3)"), vec!["rgb("]);
    assert_eq!(literals_in("RGBA(0, 0, 0, .5)"), vec!["RGBA("]);
    assert_eq!(literals_in("oklch(0.6 0.1 30)"), vec!["oklch("]);
    assert!(
        literals_in("color-mix(in srgb, var(--color-xfire-accent) 30%, transparent)").is_empty()
    );
    assert!(literals_in("var(--color-xfire-bg-dark)").is_empty());
    // Not colours: a five-digit run, an id-like word, a fragment.
    assert!(literals_in("#12345 url(#grad) #main-content").is_empty());
}

#[test]
fn a_seeded_literal_in_a_theme_part_fails() {
    let css = "@layer components {\n  .a {\n    color: var(--color-xfire-text);\n    background: rgba(0, 0, 0, 0.6);\n  }\n}";
    let hits = scan_css("src/styles/theme/99-x.css", css);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        hits[0].starts_with("src/styles/theme/99-x.css:4  rgba("),
        "{hits:?}"
    );
}

#[test]
fn finds_named_colours_as_whole_words_only() {
    assert_eq!(
        named_in("color-mix(in srgb, white 5%, transparent)"),
        vec!["white"]
    );
    assert_eq!(named_in("1px solid Black"), vec!["Black"]);
    assert_eq!(
        named_in("color-mix(in srgb, var(--color-xfire-accent) 80%, black)"),
        vec!["black"]
    );
    // Not colours: custom-property names, quoted font names, the
    // keywords without a colour, and the system colours.
    assert!(named_in("var(--color-xfire-red-tan) \"Tan Sans\", 'navy'").is_empty());
    assert!(named_in("transparent currentColor inherit initial unset").is_empty());
    assert!(named_in("1px solid CanvasText; background: Highlight").is_empty());
}

#[test]
fn the_token_blocks_may_hold_literals_and_nothing_else_in_global_may() {
    let css = "@theme {\n  --color-xfire-bg: #0a0a0a;\n}\n@layer sec { :root { --sec-x: #fff; } }\n@layer base {\n  body { color: #e7e7e7; }\n}\n/* #abc in a comment */\n@layer xfire-defaults { :root[data-skin-base=\"light\"] { --color-xfire-overlay: #000000; } }\n@layer a11y {\n  @media (prefers-contrast: more) {\n    :root { --color-xfire-border: rgba(255, 255, 255, 0.55); --color-xfire-x: white; }\n  }\n}";
    let in_global = scan_css(TOKEN_FILE, css);
    assert_eq!(in_global.len(), 3, "{in_global:?}");
    assert!(in_global[0].contains(":6  #e7e7e7"), "{in_global:?}");
    // The a11y overrides are not a token block.
    assert!(in_global[1].contains(":12  rgba("), "{in_global:?}");
    assert!(in_global[2].contains(":12  white"), "{in_global:?}");
    // The same token declarations anywhere else are literals.
    assert_eq!(scan_css("src/styles/theme/00-x.css", css).len(), 6);
}

#[test]
fn a_seeded_named_colour_in_a_theme_part_fails() {
    let css = "@layer components {\n  .a:hover {\n    background: color-mix(in srgb, white 5%, transparent);\n  }\n  .b { background: black; border: 1px solid transparent; }\n}";
    let hits = scan_css("src/styles/theme/99-x.css", css);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits[0].contains(":3  white  in `background`"), "{hits:?}");
    assert!(hits[1].contains(":5  black"), "{hits:?}");
}

#[test]
fn non_css_files_are_scanned_whole() {
    assert_eq!(
        scan_other("src/styles/regions.toml", "'.a' = { on = [\"#fff\"] }").len(),
        1
    );
    assert!(scan_other(
        "src/styles/regions.toml",
        "'.a' = { on = [\"--color-xfire-bg\"] }"
    )
    .is_empty());
}
