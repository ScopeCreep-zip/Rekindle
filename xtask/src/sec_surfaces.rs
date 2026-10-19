//! `check-sec-surfaces` (plan E7.7.0): security surfaces are unskinnable.
//!
//! A skin is colour and bitmaps (E7.7.1). If a component that asks for a
//! deliberate decision drew its buttons with the themeable `.form-btn-*`
//! classes, a colour-only skin could make a destructive commit look like a
//! safe one (`IntentDialog` used to swap `.form-btn-danger` and
//! `.form-btn-primary` on one element). So:
//!
//! - every component in [`SURFACES`] exists, uses no `form-btn-*` class and
//!   reads no custom property but a reserved `--sec-*` one;
//! - every rule in `@layer sec` reads only `--sec-*` tokens and declares no
//!   other custom property;
//! - a `--sec-*` token is declared only inside `@layer sec` (so it is never
//!   in `@theme`, the themeable set).

use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::css;

/// The security-surface components. The incoming session reset is a Host
/// review rendered by `IntentDialog` (plan E2.1); E7.7.2's skin-error
/// banner and its keep-or-revert control join this list in their step.
pub(crate) const SURFACES: &[&str] = &[
    "src/components/common/IntentDialog.tsx",
    "src/components/buddy-list/DeepLinkConsentDialog.tsx",
    "src/components/settings/AddDeviceModal.tsx",
];

const SEC_LAYER: &str = "sec";

/// Problems in one security-surface component.
fn scan_component(rel: &str, src: &str) -> Vec<String> {
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
    for rel in SURFACES {
        let path = root.join(rel);
        if !path.exists() {
            hits.push(format!(
                "{rel}  is listed as a security surface but does not exist"
            ));
            continue;
        }
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        hits.extend(scan_component(rel, &src));
    }
    for (rel, src) in css::stylesheets(root)? {
        hits.extend(scan_stylesheet(&rel, &src));
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} security-surface violation(s). Security surfaces use the \
         `.sec-btn-*` classes and the reserved `--sec-*` tokens of \
         `@layer sec` (src/styles/theme/16-security-surfaces.css).",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_with_a_form_button_or_a_theme_token_fails() {
        let ok = r#"<button class="sec-btn-commit-danger" data-role="destructive">Delete</button>
            <rect class="status-dot-cutout" />"#;
        assert!(scan_component("D.tsx", ok).is_empty());
        let swap =
            r#"<button class={warning() ? "form-btn-danger" : "form-btn-primary"}>Go</button>"#;
        assert_eq!(scan_component("D.tsx", swap).len(), 1);
        let token = r#"<div style={{ color: "var(--color-xfire-text)" }} />"#;
        let hits = scan_component("D.tsx", token);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("--color-xfire-text"), "{hits:?}");
        assert!(scan_component("D.tsx", r#"<i style="fill: var(--sec-focus)" />"#).is_empty());
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
