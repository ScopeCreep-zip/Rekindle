//! `check-motion` (2026-10-20 audit, O-4): no motion reaches a user who
//! asked the OS to reduce it (WCAG 2.3.3; macOS "Reduce motion", Windows
//! "Show animations", GNOME "Animations").
//!
//! The motion tokens `--motion-duration-*` (and `--sec-motion-duration` in
//! `@layer sec`) are `0ms` unless `prefers-reduced-motion: no-preference`
//! (`global.css`, `@layer a11y`), so a transition that reads one collapses
//! to instant for that user with no rule of its own. Over every stylesheet
//! under `src/styles/` this gate requires:
//!
//! - every `transition`, `transition-duration` and `transition-delay`
//!   reads a motion token and writes no literal time (`0.15s`, `80ms`);
//! - every `animation` or `animation-duration` either sits inside
//!   `@media (prefers-reduced-motion: no-preference)`, reads a motion token
//!   with no literal time, is `none`, or runs only `@keyframes` that change
//!   nothing but `opacity` (a fade does not move: Apple's Reduce Motion
//!   swaps slides for cross-fades). The `animationend` a component waits
//!   for still fires.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::css::{self, Block};

const GUARD: &str = "prefers-reduced-motion:no-preference";
const TRANSITION_PROPS: &[&str] = &["transition", "transition-duration", "transition-delay"];
const ANIMATION_PROPS: &[&str] = &["animation", "animation-duration"];

fn is_motion_token(name: &str) -> bool {
    name.starts_with("--motion-duration-") || name == "--sec-motion-duration"
}

/// Whether a value writes a time (`0.15s`, `80ms`, `2s`).
fn has_time_literal(value: &str) -> bool {
    value
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')'))
        .any(|w| {
            let num = w
                .strip_suffix("ms")
                .or_else(|| w.strip_suffix('s'))
                .unwrap_or("");
            !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
}

fn reads_motion_token(value: &str) -> bool {
    css::var_refs(value).iter().any(|n| is_motion_token(n))
}

/// Every `@keyframes` name with the properties its steps set.
pub(crate) fn keyframes(sheets: &[(String, String)]) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_, src) in sheets {
        for block in css::blocks(src) {
            if let Some(name) = block.keyframes_of() {
                out.entry(name.to_string())
                    .or_default()
                    .extend(block.decls.iter().map(|d| d.prop.clone()));
            }
        }
    }
    out
}

/// Whether every keyframes an `animation` value names changes only opacity.
fn fades_only(value: &str, frames: &BTreeMap<String, BTreeSet<String>>) -> bool {
    let named: Vec<&BTreeSet<String>> = value
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter_map(|w| frames.get(w))
        .collect();
    !named.is_empty()
        && named
            .iter()
            .all(|props| props.iter().all(|p| p == "opacity"))
}

fn scan_block(
    rel: &str,
    block: &Block,
    frames: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let mut hits = Vec::new();
    if block.in_keyframes() {
        return hits;
    }
    for decl in &block.decls {
        let value = decl.value.trim();
        if value == "none" {
            continue;
        }
        let prop = decl.prop.as_str();
        if TRANSITION_PROPS.contains(&prop)
            && (has_time_literal(value) || !reads_motion_token(value))
        {
            hits.push(format!(
                "{rel}:{}  `{prop}: {value}` does not collapse for reduced motion; use \
                 `var(--motion-duration-fast)` or `-base` / `-slow`",
                decl.line
            ));
        }
        let tokened = reads_motion_token(value) && !has_time_literal(value);
        let still =
            block.under(GUARD) || tokened || (prop == "animation" && fades_only(value, frames));
        if ANIMATION_PROPS.contains(&prop) && !still {
            hits.push(format!(
                "{rel}:{}  `{prop}: {value}` moves for a reduced-motion user; put it in \
                 `@media (prefers-reduced-motion: no-preference)` and give that user a \
                 fade or a static state",
                decl.line
            ));
        }
    }
    hits
}

/// The motion problems of a set of stylesheets.
pub(crate) fn scan(sheets: &[(String, String)]) -> Vec<String> {
    let frames = keyframes(sheets);
    sheets
        .iter()
        .flat_map(|(rel, src)| {
            css::blocks(src)
                .iter()
                .flat_map(|b| scan_block(rel, b, &frames))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(crate) fn check_motion(root: &Path) -> Result<()> {
    let hits = scan(&css::stylesheets(root)?);
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} motion violation(s): transitions read a `--motion-duration-*` token, \
         and animations that move run only under \
         `prefers-reduced-motion: no-preference`.",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(css: &str) -> Vec<String> {
        scan(&[("a.css".to_string(), css.to_string())])
    }

    #[test]
    fn a_literal_transition_fails_and_a_token_passes() {
        let hits = run(".a {\n  transition: color 0.15s, background 0.15s;\n}");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
        assert_eq!(
            run(".a { transition: border-color 80ms ease-out; }").len(),
            1
        );
        assert_eq!(run(".a { transition: opacity; }").len(), 1);
        assert!(run(".a { transition: color var(--motion-duration-fast), background var(--motion-duration-fast); }").is_empty());
        assert!(
            run(".a { transition: filter var(--sec-motion-duration); transition: none; }")
                .is_empty()
        );
    }

    #[test]
    fn an_unguarded_moving_animation_fails() {
        let frames = "@keyframes float { 0% { opacity: 0; transform: translateY(0); } 100% { transform: translateY(-9px); } }\n@keyframes hold { 0% { opacity: 0 } 100% { opacity: 1 } }\n";
        let moving = format!("{frames}.a {{ animation: float 2s ease-out forwards; }}");
        let hits = run(&moving);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0].contains("moves for a reduced-motion user"),
            "{hits:?}"
        );
        let guarded = format!("{frames}@media (prefers-reduced-motion: no-preference) {{ @layer components {{ .a {{ animation: float 2s; }} }} }}");
        assert!(run(&guarded).is_empty());
        let fade = format!("{frames}.a {{ animation: hold 2s linear forwards; }}");
        assert!(run(&fade).is_empty());
        let tokened =
            format!("{frames}.a {{ animation: float var(--motion-duration-base) ease-out; }}");
        assert!(run(&tokened).is_empty());
        assert_eq!(
            run(&format!("{frames}.a {{ animation-duration: 2s; }}")).len(),
            1
        );
        assert!(run(".a { animation: none; }").is_empty());
    }
    #[test]
    fn a_guarded_float_with_a_fading_hold_passes_inside_a_layer() {
        let css = "@layer components {\n  .f { animation: hold 2s linear forwards; }\n  @media (prefers-reduced-motion: no-preference) {\n    .f { animation: float 2s ease-out forwards; }\n  }\n  @keyframes hold { 0% { opacity: 1; } 100% { opacity: 0; } }\n  @keyframes float { 0% { transform: scale(0.6); } 100% { opacity: 0; } }\n}";
        assert!(run(css).is_empty(), "{:?}", run(css));
        let unguarded = css.replace("hold 2s", "float 2s");
        let hits = run(&unguarded);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
    }

    #[test]
    fn duration_and_delay_longhands_are_checked() {
        assert_eq!(
            run(".a { transition-duration: 150ms; transition-delay: 0.1s; }").len(),
            2
        );
        assert!(run(".a { transition-duration: var(--motion-duration-slow); }").is_empty());
    }
}
