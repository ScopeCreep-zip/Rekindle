//! `check-sec-surfaces` (plan E7.7.0): security surfaces are unskinnable.
//!
//! A skin is colour and bitmaps (E7.7.1). If a component that asks for a
//! deliberate decision drew its buttons with the themeable `.form-btn-*`
//! classes, a colour-only skin could make a destructive commit look like a
//! safe one (`IntentDialog` used to swap `.form-btn-danger` and
//! `.form-btn-primary` on one element); if the text the decision rests on
//! read a themeable token, a skin could paint it in its own ground's colour
//! and hide it. So each surface's **security-critical** elements (the
//! commit controls, and the text a user reads or compares before deciding:
//! the review's subject, facts, escalations and consequences, a key
//! fingerprint, a safety number, a pairing code and its QR) carry only
//! reserved `.sec-*` classes, drawn in `@layer sec` on a fixed ground. The
//! dialog chrome around them (title, frame, overlay, form fields, error
//! line) stays themeable. The gate:
//!
//! - every component in [`SURFACES`] exists, uses every security class its
//!   row names, uses no `form-btn-*` class and reads no custom property
//!   but a reserved `--sec-*` one;
//! - an element there with a `sec-*` class carries no other class, so no
//!   themeable rule reaches it;
//! - every rule anywhere whose selector names a `sec-*` class reads only
//!   `--sec-*` tokens, and a `sec-*` class is used by no component outside
//!   [`SURFACES`];
//! - every rule in `@layer sec` reads only `--sec-*` tokens and declares no
//!   other custom property;
//! - a `--sec-*` token is declared only inside `@layer sec` (so it is never
//!   in `@theme`, the themeable set).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::css;
use crate::jsx::elements;
use crate::walk_source_files;

/// The security-surface components and the security-critical classes each
/// must use. The incoming session reset is a Host review rendered by
/// `IntentDialog` (plan E2.1); the safety number and the key-trust badge
/// are what two people compare (plan E2.3); E7.7.2's skin-error banner and
/// its keep-or-revert control join this list in their step.
pub(crate) const SURFACES: &[(&str, &[&str])] = &[
    (
        "src/components/common/IntentDialog.tsx",
        &[
            "sec-review",
            "sec-review-subject",
            "sec-review-facts",
            "sec-review-escalation",
            "sec-review-meta",
            "sec-review-ack",
            "sec-btn-commit",
            "sec-btn-commit-danger",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/buddy-list/DeepLinkConsentDialog.tsx",
        &[
            "sec-review",
            "sec-review-fact",
            "sec-fingerprint",
            "sec-fingerprint-value",
            "sec-btn-commit",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/settings/AddDeviceModal.tsx",
        &[
            "sec-pairing-qr",
            "sec-pairing-code",
            "sec-btn-commit",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/profile/SafetyNumberPanel.tsx",
        &[
            "sec-safety-number",
            "sec-safety-number-digits",
            "sec-safety-number-qr",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/profile/VerifiedBadge.tsx",
        &["sec-verified-badge", "sec-key-changed-badge"],
    ),
];

const SEC_LAYER: &str = "sec";

fn is_sec_class(class: &str) -> bool {
    class.starts_with("sec-")
}

/// Problems in one security-surface component; adds the security classes
/// it uses to `used`.
fn scan_component(
    rel: &str,
    src: &str,
    required: &[&str],
    used: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut hits = Vec::new();
    for (i, line) in src.lines().enumerate() {
        if line.contains("form-btn-") {
            hits.push(format!(
                "{rel}:{}  uses a themeable `.form-btn-*` class; use `.sec-btn-*`",
                i + 1
            ));
        }
        for name in css::var_refs(line) {
            if !name.starts_with("--sec-") {
                hits.push(format!(
                    "{rel}:{}  reads the themeable token `{name}`",
                    i + 1
                ));
            }
        }
    }
    let mut here = BTreeSet::new();
    for el in elements(src) {
        let classes = el.classes();
        if !classes.iter().any(|c| is_sec_class(c)) {
            continue;
        }
        for class in &classes {
            if is_sec_class(class) {
                here.insert(class.clone());
            } else {
                hits.push(format!(
                    "{rel}:{}  <{}> is security-critical but also carries the themeable class `{class}`",
                    el.line, el.tag
                ));
            }
        }
    }
    for class in required {
        if !here.contains(*class) {
            hits.push(format!(
                "{rel}  does not use its security class `{class}` (xtask/src/sec_surfaces.rs)"
            ));
        }
    }
    used.extend(here);
    hits
}

/// A component outside [`SURFACES`] that uses a security class.
fn stray_sec_classes(rel: &str, src: &str) -> Vec<String> {
    elements(src)
        .into_iter()
        .flat_map(|el| {
            el.classes()
                .into_iter()
                .filter(|c| is_sec_class(c))
                .map(move |c| {
                    format!(
                        "{rel}:{}  uses the security class `{c}` but is not in SURFACES \
                         (xtask/src/sec_surfaces.rs)",
                        el.line
                    )
                })
        })
        .collect()
}

/// The classes a selector names (`.a:hover .b > c` → `a`, `b`).
fn selector_classes(selector: &str) -> Vec<&str> {
    selector
        .split('.')
        .skip(1)
        .map(|rest| {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .filter(|c| !c.is_empty())
        .collect()
}

/// Rules outside `@layer sec` that style a security class and read a
/// themeable token (inside it, [`scan_stylesheet`] reports them).
fn themed_sec_rules(rel: &str, src: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for block in css::blocks(src) {
        if !block.is_rule() || block.in_layer(SEC_LAYER) {
            continue;
        }
        let Some(class) = block
            .selectors()
            .iter()
            .flat_map(|s| {
                selector_classes(s)
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .find(|c| is_sec_class(c))
        else {
            continue;
        };
        for decl in &block.decls {
            for name in css::var_refs(&decl.value) {
                if !name.starts_with("--sec-") {
                    hits.push(format!(
                        "{rel}:{}  a rule on the security class `.{class}` reads the themeable `{name}`",
                        decl.line
                    ));
                }
            }
        }
    }
    hits
}

/// Problems in one stylesheet's `@layer sec` and `--sec-*` declarations.
fn scan_stylesheet(rel: &str, src: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for block in css::blocks(src) {
        let in_sec = block.in_layer(SEC_LAYER);
        for decl in &block.decls {
            let declares_sec = decl.prop.starts_with("--sec-");
            if declares_sec && !in_sec {
                hits.push(format!(
                    "{rel}:{}  `{}` is declared outside `@layer sec`",
                    decl.line, decl.prop
                ));
            }
            if !in_sec {
                continue;
            }
            if decl.prop.starts_with("--") && !declares_sec {
                hits.push(format!(
                    "{rel}:{}  `@layer sec` declares the non-reserved `{}`",
                    decl.line, decl.prop
                ));
            }
            for name in css::var_refs(&decl.value) {
                if !name.starts_with("--sec-") {
                    hits.push(format!(
                        "{rel}:{}  `@layer sec` reads the themeable `{name}`",
                        decl.line
                    ));
                }
            }
        }
    }
    hits
}

pub(crate) fn check_sec_surfaces(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    let mut used = BTreeSet::new();
    for (rel, required) in SURFACES {
        let path = root.join(rel);
        if !path.exists() {
            hits.push(format!(
                "{rel}  is listed as a security surface but does not exist"
            ));
            continue;
        }
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        hits.extend(scan_component(rel, &src, required, &mut used));
    }
    for path in walk_source_files(&root.join("src"), &["tsx"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if SURFACES.iter().any(|(s, _)| *s == rel) {
            continue;
        }
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        hits.extend(stray_sec_classes(&rel, &src));
    }
    for (rel, src) in css::stylesheets(root)? {
        hits.extend(scan_stylesheet(&rel, &src));
        hits.extend(themed_sec_rules(&rel, &src));
    }
    if hits.is_empty() {
        println!(
            "  {} security classes on {} surfaces",
            used.len(),
            SURFACES.len()
        );
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} security-surface violation(s). Security-critical elements carry \
         only the `.sec-*` classes of `@layer sec` \
         (src/styles/theme/16-security-surfaces.css), which read only the \
         reserved `--sec-*` tokens.",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str, required: &[&str]) -> Vec<String> {
        scan_component("D.tsx", src, required, &mut BTreeSet::new())
    }

    #[test]
    fn a_surface_with_a_form_button_or_a_theme_token_fails() {
        let ok = r#"<button class="sec-btn-commit-danger" data-role="destructive">Delete</button>
            <rect class="status-dot-cutout" />"#;
        assert!(scan(ok, &["sec-btn-commit-danger"]).is_empty());
        let swap =
            r#"<button class={warning() ? "form-btn-danger" : "form-btn-primary"}>Go</button>"#;
        assert_eq!(scan(swap, &[]).len(), 1);
        let token = r#"<div style={{ color: "var(--color-xfire-text)" }} />"#;
        let hits = scan(token, &[]);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("--color-xfire-text"), "{hits:?}");
        assert!(scan(r#"<i style="fill: var(--sec-focus)" />"#, &[]).is_empty());
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
        assert_eq!(
            stray_sec_classes("X.tsx", r#"<b class="sec-review" />"#).len(),
            1
        );
        assert!(stray_sec_classes("X.tsx", r#"<b class="review" />"#).is_empty());
    }

    #[test]
    fn a_themed_rule_on_a_security_class_fails_in_any_layer() {
        let css = "@layer components {\n  .consent-dialog .sec-fingerprint-value { color: var(--color-xfire-text); }\n  .sec-review-ack input { margin: 0; }\n  .pairing { background: var(--sec-qr-backing); }\n}";
        let hits = themed_sec_rules("a.css", css);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
        assert!(hits[0].contains("`.sec-fingerprint-value`"), "{hits:?}");
        assert_eq!(selector_classes(".a:hover .b-c > d"), vec!["a", "b-c"]);
    }

    #[test]
    fn the_sec_layer_reads_and_declares_only_reserved_tokens() {
        let ok = "@layer sec {\n  :root { --sec-commit-bg: #1565a8; }\n  .sec-btn-commit { background: var(--sec-commit-bg); }\n}\n.qr { background: var(--sec-qr-backing); }";
        assert!(scan_stylesheet("a.css", ok).is_empty());
        let themed = "@layer sec {\n  .sec-btn-commit { color: var(--color-xfire-text); }\n}";
        let hits = scan_stylesheet("a.css", themed);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
        let other = "@layer sec { :root { --color-xfire-accent: #f00; } }";
        assert_eq!(scan_stylesheet("a.css", other).len(), 1);
    }

    #[test]
    fn a_reserved_token_outside_the_sec_layer_fails() {
        let themeable = "@theme {\n  --sec-commit-bg: #f00;\n}";
        let hits = scan_stylesheet("g.css", themeable);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("outside `@layer sec`"), "{hits:?}");
    }
}
