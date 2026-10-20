//! The stylesheet half of `check-sec-surfaces` (plan E7.7.0; closed against
//! the bypasses the 2026-10-20 audit found, O-3). Over every stylesheet
//! under `src/styles/`:
//!
//! - a `--sec-*` token is declared only inside `@layer sec` (so it is never
//!   in `@theme`, the themeable set), and every rule in `@layer sec` reads
//!   only `--sec-*` tokens and declares no other custom property;
//! - no rule outside `@layer sec` names a `sec-*` class, as a class or in
//!   an attribute selector: only `@layer sec`, last in the cascade, styles a
//!   security surface;
//! - a rule that names one never fades or hides it: no `opacity` below 1,
//!   no `visibility` but `visible`, no `display: none` or `contents`, no
//!   `content-visibility` but `visible`, no `filter: opacity(…)`, and no
//!   `transparent` colour (a disabled control is drawn with its own
//!   `--sec-disabled-*` tokens);
//! - `!important` appears only in `@layer a11y` and `@layer sec`: CSS
//!   Cascade 5 reverses the layer order for important declarations, so one
//!   in a lower layer would beat every normal declaration of `@layer sec`;
//! - a focus ring drawn with `--sec-focus` brings its own ground: the same
//!   rule paints `--sec-focus-ground` around it (`box-shadow`, wider than
//!   the ring and its offset), so the ring never depends for its contrast
//!   on the themeable surface it sits on (WCAG technique C40, a two-colour
//!   focus indicator).

use crate::css::{self, Block};
use crate::sec_surfaces::sec_names;

const SEC_LAYER: &str = "sec";
/// The layers whose declarations may be `!important`.
const IMPORTANT_LAYERS: &[&str] = &["a11y", "sec"];
const FOCUS: &str = "--sec-focus";
const FOCUS_GROUND: &str = "--sec-focus-ground";

/// The reserved-token problems of one block.
fn tokens(rel: &str, block: &Block) -> Vec<String> {
    let mut hits = Vec::new();
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
    hits
}

fn has_word(value: &str, word: &str) -> bool {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .any(|w| w == word)
}

/// How a declaration in a rule on a security class fades or hides it.
fn hides(prop: &str, value: &str) -> Option<&'static str> {
    let v = value
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    match prop {
        "opacity" if v != "1" => Some("fades it (`opacity` below 1)"),
        "visibility" if v != "visible" => Some("hides it (`visibility`)"),
        "display" if v == "none" || v == "contents" => Some("removes its box (`display`)"),
        "content-visibility" if v != "visible" => Some("hides it (`content-visibility`)"),
        "filter" | "backdrop-filter" if v.contains("opacity(") => {
            Some("fades it (`filter: opacity()`)")
        }
        _ if has_word(&v, "transparent") => Some("paints it `transparent`"),
        _ => None,
    }
}

/// The problems of a rule whose selector names a security class.
fn sec_rule(rel: &str, block: &Block, class: &str) -> Vec<String> {
    let mut hits = Vec::new();
    if !block.in_layer(SEC_LAYER) {
        hits.push(format!(
            "{rel}:{}  a rule outside `@layer sec` names the security class `{class}`; \
             only `@layer sec` styles a security surface",
            block.line
        ));
    }
    for decl in &block.decls {
        if let Some(how) = hides(&decl.prop, &decl.value) {
            hits.push(format!(
                "{rel}:{}  a rule on the security class `{class}` {how}",
                decl.line
            ));
        }
    }
    hits
}

fn important(rel: &str, block: &Block) -> Vec<String> {
    if IMPORTANT_LAYERS.iter().any(|l| block.in_layer(l)) {
        return Vec::new();
    }
    block
        .decls
        .iter()
        .filter(|d| d.value.contains("!important"))
        .map(|d| {
            format!(
                "{rel}:{}  `{}` is `!important` outside `@layer a11y` and `@layer sec`, \
                 so it would beat `@layer sec`; raise the selector instead",
                d.line, d.prop
            )
        })
        .collect()
}

fn reads(block: &Block, props: &[&str], token: &str) -> bool {
    block
        .decls
        .iter()
        .filter(|d| props.contains(&d.prop.as_str()))
        .any(|d| css::var_refs(&d.value).iter().any(|n| n == token))
}

fn focus_ground(rel: &str, block: &Block) -> Vec<String> {
    if !reads(block, &["outline", "outline-color"], FOCUS)
        || reads(block, &["box-shadow"], FOCUS_GROUND)
    {
        return Vec::new();
    }
    vec![format!(
        "{rel}:{}  draws the `{FOCUS}` ring on whatever ground surrounds it; give it its \
         own (`box-shadow: 0 0 0 6px var({FOCUS_GROUND})` in the same rule)",
        block.line
    )]
}

/// Every security-surface problem in one stylesheet.
pub(crate) fn scan(rel: &str, src: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for block in css::blocks(src) {
        hits.extend(tokens(rel, &block));
        hits.extend(important(rel, &block));
        hits.extend(focus_ground(rel, &block));
        if !block.is_rule() || block.in_keyframes() {
            continue;
        }
        if let Some((_, class)) = sec_names(&block.prelude).into_iter().next() {
            hits.extend(sec_rule(rel, &block, &class));
        }
    }
    hits
}

#[cfg(test)]
mod tests {
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
        let bare = "@layer sec {\n  .sec-btn-commit:focus-visible { outline: 2px solid var(--sec-focus); }\n}";
        let hits = scan("a.css", bare);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("own (`box-shadow"), "{hits:?}");
        let grounded = "@layer sec { .sec-btn-commit:focus-visible { outline: 2px solid var(--sec-focus); box-shadow: 0 0 0 6px var(--sec-focus-ground); } }";
        assert!(scan("a.css", grounded).is_empty());
    }
}
