use super::*;

fn has(hits: &[String], text: &str) -> bool {
    hits.iter().any(|h| h.contains(text))
}

#[test]
fn the_sec_layer_reads_and_declares_only_reserved_tokens() {
    let ok = "@layer sec {\n  :root { --sec-commit-bg: #1565a8; }\n  .sec-btn-commit { background: var(--sec-commit-bg); }\n}\n.qr { background: var(--sec-qr-backing); }";
    assert!(scan("a.css", ok).is_empty(), "{:?}", scan("a.css", ok));
    let themed = "@layer sec {\n  .sec-btn-commit { color: var(--color-xfire-text); }\n}";
    let hits = scan("a.css", themed);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
    let other = "@layer sec { :root { --color-xfire-accent: #f00; } }";
    assert_eq!(scan("a.css", other).len(), 1);
}

#[test]
fn a_reserved_token_outside_the_sec_layer_fails() {
    let themeable = "@theme {\n  --sec-commit-bg: #f00;\n}";
    let hits = scan("g.css", themeable);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].contains("outside `@layer sec`"), "{hits:?}");
}

#[test]
fn a_rule_outside_the_sec_layer_naming_a_security_class_fails() {
    let css = "@layer components {\n  .consent-dialog .sec-fingerprint-value { margin: 0; }\n  [class^=\"sec-\"] { display: block; }\n  .spec-x { margin: 0; }\n}";
    let hits = scan("a.css", css);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(
        hits[0].starts_with("a.css:2 ") && has(&hits, "`sec-fingerprint-value`"),
        "{hits:?}"
    );
    assert!(hits[1].starts_with("a.css:3 "), "{hits:?}");
}

#[test]
fn a_security_rule_that_fades_or_hides_fails() {
    for (decl, why) in [
        ("opacity: 0.5", "opacity"),
        ("visibility: hidden", "visibility"),
        ("display: none", "display"),
        ("display: contents", "display"),
        ("content-visibility: hidden", "content-visibility"),
        ("filter: opacity(0)", "filter: opacity()"),
        ("color: transparent", "transparent"),
        (
            "background: color-mix(in srgb, var(--sec-x) 10%, transparent)",
            "transparent",
        ),
    ] {
        let css = format!("@layer sec {{ .sec-btn-commit:disabled {{ {decl}; }} }}");
        let hits = scan("a.css", &css);
        assert_eq!(hits.len(), 1, "{decl}: {hits:?}");
        assert!(hits[0].contains(why), "{decl}: {hits:?}");
    }
    let ok = "@layer sec { :where([class^=\"sec-\"]) { opacity: 1; visibility: visible; } .sec-review { display: flex; } }";
    assert!(scan("a.css", ok).is_empty(), "{:?}", scan("a.css", ok));
}

#[test]
fn important_is_refused_outside_a11y_and_sec() {
    let css = "@layer components {\n  .a { color: var(--x) !important; }\n}\n@layer a11y { @media (forced-colors: active) { .b { color: CanvasText !important; } } }\n@layer sec { .sec-c { margin: 0 !important; } }";
    let hits = scan("a.css", css);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
}

#[test]
fn a_focus_ring_without_its_own_ground_fails() {
    let bare =
        "@layer sec {\n  .sec-btn-commit:focus-visible { outline: 2px solid var(--sec-focus); }\n}";
    let hits = scan("a.css", bare);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("own (`box-shadow"), "{hits:?}");
    let grounded = "@layer sec { .sec-btn-commit:focus-visible { outline: 2px solid var(--sec-focus); box-shadow: 0 0 0 6px var(--sec-focus-ground); } }";
    assert!(scan("a.css", grounded).is_empty());
}
