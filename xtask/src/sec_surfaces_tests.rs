use super::*;

fn scan(src: &str, required: &[&str]) -> Vec<String> {
    scan_component(
        "src/components/profile/D.tsx",
        src,
        required,
        &mut BTreeSet::new(),
    )
}

fn has(hits: &[String], text: &str) -> bool {
    hits.iter().any(|h| h.contains(text))
}

#[test]
fn a_surface_with_a_form_button_or_a_theme_token_fails() {
    let ok = r#"<button class="sec-btn-commit-danger" data-role="destructive">Delete</button>
            <rect class="status-dot-cutout" />"#;
    assert!(scan(ok, &["sec-btn-commit-danger"]).is_empty());
    let swap = r#"<button class={warning() ? "form-btn-danger" : "form-btn-primary"}>Go</button>"#;
    assert_eq!(scan(swap, &[]).len(), 1);
    let token = r#"<div data-c="var(--color-xfire-text)" />"#;
    let hits = scan(token, &[]);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].contains("--color-xfire-text"), "{hits:?}");
    assert!(scan(r#"<i data-x="fill: var(--sec-focus)" />"#, &[]).is_empty());
}

#[test]
fn security_text_carries_only_its_security_classes() {
    let mixed = r#"<p class="sec-fingerprint consent-dialog-key">Key <code class="sec-fingerprint-value">ab</code></p>"#;
    let hits = scan(mixed, &["sec-fingerprint", "sec-fingerprint-value"]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("themeable class `consent-dialog-key`"));
    let gone = r#"<p class="consent-dialog-key">Key <code>ab</code></p>"#;
    let hits = scan(gone, &["sec-fingerprint"]);
    assert!(hits[0].contains("does not use its security class `sec-fingerprint`"));
}

#[test]
fn every_element_inside_a_security_element_is_checked() {
    let icon = r#"<span class="sec-verified-badge"><span class="nf-icon" aria-hidden="true">x</span> Verified</span>"#;
    let hits = scan(icon, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        has(&hits, "sits in the security-critical <span>"),
        "{hits:?}"
    );
    assert!(has(&hits, "themeable class `nf-icon`"), "{hits:?}");
    let deep = r#"<div class="sec-review"><Show when={x()}><ul><li class={dim()}>a</li></ul></Show></div>"#;
    assert!(has(
        &scan(deep, &[]),
        "chooses its class with an expression"
    ));
    let hidden = r#"<div class="sec-review"><p style={{ opacity: 0 }}>fact</p></div>"#;
    assert!(has(&scan(hidden, &[]), "carries `style`"));
    let component = r#"<div class="sec-review"><Tooltip text="x" /></div>"#;
    assert!(has(&scan(component, &[]), "<Tooltip> sits in"));
    let listed = "import VerifiedBadge from \"./VerifiedBadge\";\n<div class=\"sec-safety-number\"><VerifiedBadge view={v()} /><span aria-hidden=\"true\">1</span></div>";
    let hits = scan(listed, &[]);
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn an_error_boundary_draws_nothing_but_its_fallback_is_checked() {
    let fixed = r#"<section class="sec-safety-number"><ErrorBoundary fallback={<p class="sec-safety-number-hint">No code</p>}><svg class="sec-safety-number-qr" /></ErrorBoundary></section>"#;
    let hits = scan(fixed, &[]);
    assert!(hits.is_empty(), "{hits:?}");
    let themed = r#"<section class="sec-safety-number"><ErrorBoundary fallback={<p class={dim()}>No code</p>}><svg class="sec-safety-number-qr" /></ErrorBoundary></section>"#;
    assert!(has(
        &scan(themed, &[]),
        "chooses its class with an expression"
    ));
}

#[test]
fn class_lists_templates_spreads_and_hiding_attributes_fail() {
    let list = r#"<div class="sec-review" classList={{ dim: off() }} />"#;
    assert!(has(&scan(list, &[]), "carries `classList`"));
    let keyed = r#"<div classList={{ "sec-review": on() }} />"#;
    assert!(has(&scan(keyed, &[]), "carries `classList`"));
    let template = r"<div class={`sec-review ${extra()}`} />";
    assert!(has(&scan(template, &[]), "template literal"));
    let spread = r#"<button class="sec-btn-commit" {...rest}>Go</button>"#;
    assert!(has(&scan(spread, &[]), "spreads props"));
    let hidden = r#"<p class="sec-review-fact" hidden={!x()}>a</p>"#;
    assert!(has(&scan(hidden, &[]), "carries `hidden`"));
}

#[test]
fn a_security_name_outside_a_static_class_attribute_fails() {
    let helper = "const cls = () => \"sec-review\";\n// sec-review in a comment is fine\n<div class={cls()} />";
    let hits = scan(helper, &[]);
    assert!(
        has(
            &hits,
            "D.tsx:1  names the security class `sec-review` outside"
        ),
        "{hits:?}"
    );
    assert!(!has(&hits, "D.tsx:2 "), "{hits:?}");
    let ok = "/** draws `.sec-review` */\n<div class=\"sec-review\">a</div>";
    assert!(scan(ok, &["sec-review"]).is_empty());
}

#[test]
fn any_other_file_naming_a_security_class_fails() {
    assert_eq!(
        stray_sec_names("X.tsx", r#"<b class="sec-review" />"#).len(),
        1
    );
    let ts = "el.classList.add(\"sec-btn-commit\");\n// sec-x\nconst v = \"--sec-focus\";";
    let hits = stray_sec_names("x.ts", ts);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with("x.ts:1 "), "{hits:?}");
    assert!(stray_sec_names("X.tsx", r#"<b class="review spec-x" />"#).is_empty());
}

#[test]
fn imports_resolve_to_listed_surfaces_only() {
    let src =
        "import VerifiedBadge from \"./VerifiedBadge\";\nimport Modal from \"../common/Modal\";";
    let names = surface_imports("src/components/profile/SafetyNumberPanel.tsx", src);
    assert_eq!(names.into_iter().collect::<Vec<_>>(), vec!["VerifiedBadge"]);
}
