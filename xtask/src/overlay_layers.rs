//! `check-overlay-layers` (plan U1, ADR 0019; found by the E7.14 real-app
//! layout suite): the U1 review is always the top overlay.
//!
//! A review once drew behind the "Create Community" form that raised it:
//! both portaled Kobalte Dialogs read `--z-modal`, so the stacking followed
//! the portal DOM order. The review is the safety step, so it has its own
//! layer, `--z-review-backdrop` / `--z-review`, defined once in
//! `src/styles/global.css`. Over every stylesheet under `src/styles/` this
//! gate requires:
//!
//! - both review tokens are defined exactly once, as integers, the content
//!   above its backdrop;
//! - every other `--z-*` token (the skip link excepted: it cannot take focus
//!   while the review traps it) and every literal `z-index` sits below the
//!   review's backdrop;
//! - only `.intent-overlay` / `.intent-overlay-positioner` rules read the
//!   review tokens, and both read theirs;
//! - `IntentDialog.tsx` carries both classes.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::css;

const BACKDROP: &str = "--z-review-backdrop";
const CONTENT: &str = "--z-review";
const EXEMPT: &str = "--z-skip-link";
const INTENT_DIALOG: &str = "src/components/common/IntentDialog.tsx";

fn is_review_token(name: &str) -> bool {
    name == BACKDROP || name == CONTENT
}

/// The `z-index`/token value as an integer, if it is a plain number.
fn integer(value: &str) -> Option<i64> {
    value.trim().parse().ok()
}

fn problems(sheets: &[(String, String)], intent_dialog: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut tokens: BTreeMap<String, Vec<(String, usize, Option<i64>)>> = BTreeMap::new();
    let mut literals = Vec::new();
    let mut readers: BTreeMap<&str, bool> = BTreeMap::new();
    for (rel, src) in sheets {
        for block in css::blocks(&css::strip_comments(src)) {
            for decl in &block.decls {
                if decl.prop.starts_with("--z-") {
                    tokens.entry(decl.prop.clone()).or_default().push((
                        rel.clone(),
                        decl.line,
                        integer(&decl.value),
                    ));
                }
                if decl.prop == "z-index" {
                    if let Some(n) = integer(&decl.value) {
                        literals.push((rel.clone(), decl.line, n));
                    }
                }
                for name in css::var_refs(&decl.value) {
                    if !is_review_token(&name) {
                        continue;
                    }
                    let selectors = block.selectors();
                    let intent =
                        block.is_rule() && selectors.iter().all(|s| s.contains(".intent-overlay"));
                    if !intent {
                        out.push(format!(
                            "{rel}:{}: `{}` reads {name}; only `.intent-overlay*` may",
                            decl.line, block.prelude
                        ));
                        continue;
                    }
                    for s in &selectors {
                        if s.ends_with(".intent-overlay") && name == BACKDROP {
                            readers.insert(BACKDROP, true);
                        }
                        if s.ends_with(".intent-overlay-positioner") && name == CONTENT {
                            readers.insert(CONTENT, true);
                        }
                    }
                }
            }
        }
    }
    let single = |name: &str, out: &mut Vec<String>| -> Option<i64> {
        match tokens.get(name).map(Vec::as_slice) {
            Some([(_, _, Some(n))]) => Some(*n),
            Some([(rel, line, None)]) => {
                out.push(format!("{rel}:{line}: {name} is not an integer"));
                None
            }
            Some(defs) if defs.len() > 1 => {
                out.push(format!(
                    "{name} is defined {} times; define it once",
                    defs.len()
                ));
                None
            }
            _ => {
                out.push(format!("{name} is not defined"));
                None
            }
        }
    };
    let (Some(backdrop), Some(content)) = (single(BACKDROP, &mut out), single(CONTENT, &mut out))
    else {
        return out;
    };
    if content <= backdrop {
        out.push(format!(
            "{CONTENT} ({content}) must sit above {BACKDROP} ({backdrop})"
        ));
    }
    for (name, defs) in &tokens {
        if is_review_token(name) || name == EXEMPT {
            continue;
        }
        for (rel, line, value) in defs {
            if value.is_none_or(|n| n >= backdrop) {
                out.push(format!(
                    "{rel}:{line}: {name} must be an integer below {BACKDROP} ({backdrop})"
                ));
            }
        }
    }
    for (rel, line, n) in literals {
        if n >= backdrop {
            out.push(format!(
                "{rel}:{line}: z-index {n} reaches the U1 review's layer"
            ));
        }
    }
    for name in [BACKDROP, CONTENT] {
        if !readers.contains_key(name) {
            let class = if name == BACKDROP {
                ".intent-overlay"
            } else {
                ".intent-overlay-positioner"
            };
            out.push(format!("no `{class}` rule reads {name}"));
        }
    }
    for class in ["intent-overlay\"", "intent-overlay-positioner\""] {
        if !intent_dialog.contains(class) {
            out.push(format!(
                "{INTENT_DIALOG} does not carry `{}`",
                class.trim_end_matches('"')
            ));
        }
    }
    out
}

pub(crate) fn check_overlay_layers(root: &Path) -> Result<()> {
    let sheets = css::stylesheets(root)?;
    let dialog = std::fs::read_to_string(root.join(INTENT_DIALOG))
        .with_context(|| format!("reading {INTENT_DIALOG}"))?;
    let found = problems(&sheets, &dialog);
    if found.is_empty() {
        println!("check-overlay-layers: the U1 review is the top overlay layer");
        return Ok(());
    }
    for p in &found {
        eprintln!("  {p}");
    }
    Err(anyhow!(
        "check-overlay-layers: {} problem(s); the U1 review must draw above every overlay",
        found.len()
    ))
}

#[cfg(test)]
mod tests {
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
}
