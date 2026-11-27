use super::*;

const TOKENS: &str = ":root { --z-modal: 510; --z-toast: 600; --z-review-backdrop: 9000; \
                          --z-review: 9010; --z-skip-link: 9999; }";
const RULES: &str = ".modal-overlay.intent-overlay { z-index: var(--z-review-backdrop); }\n\
                         .modal-overlay-positioner.intent-overlay-positioner { z-index: var(--z-review); }";
const DIALOG: &str = r#"<Dialog.Overlay class="modal-overlay intent-overlay" />
        <div class="modal-overlay-positioner intent-overlay-positioner">"#;

fn run(extra: &str, dialog: &str) -> Vec<String> {
    let sheets = vec![
        ("global.css".to_owned(), TOKENS.to_owned()),
        ("theme.css".to_owned(), format!("{RULES}\n{extra}")),
    ];
    problems(&sheets, dialog)
}

#[test]
fn the_review_on_its_own_top_layer_passes() {
    assert!(run(".toast { z-index: 80; }", DIALOG).is_empty());
}

#[test]
fn the_review_on_the_modal_layer_fails() {
    // The bug: the review read the shared modal layer.
    let sheets = vec![
        (
            "global.css".to_owned(),
            ":root { --z-modal: 510; }".to_owned(),
        ),
        (
            "theme.css".to_owned(),
            ".modal-overlay { z-index: var(--z-modal); }".to_owned(),
        ),
    ];
    let p = problems(&sheets, r#"class="modal-overlay""#);
    assert!(
        p.iter()
            .any(|s| s.contains("--z-review-backdrop is not defined")),
        "{p:?}"
    );
}

#[test]
fn a_layer_at_or_above_the_review_fails() {
    let p = run(
        ":root { --z-banner: 9000; }\n.menu { z-index: 9500; }",
        DIALOG,
    );
    assert!(p.iter().any(|s| s.contains("--z-banner must be")), "{p:?}");
    assert!(p.iter().any(|s| s.contains("z-index 9500")), "{p:?}");
}

#[test]
fn another_rule_reading_the_review_layer_fails() {
    let p = run(".toast { z-index: var(--z-review); }", DIALOG);
    assert!(
        p.iter().any(|s| s.contains("only `.intent-overlay*`")),
        "{p:?}"
    );
}

#[test]
fn a_dialog_without_the_classes_or_a_second_definition_fails() {
    let p = run(":root { --z-review: 9020; }", r#"class="modal-overlay""#);
    assert!(p.iter().any(|s| s.contains("defined 2 times")), "{p:?}");
    let p = run("", r#"class="modal-overlay""#);
    assert!(p.iter().any(|s| s.contains("does not carry")), "{p:?}");
}
