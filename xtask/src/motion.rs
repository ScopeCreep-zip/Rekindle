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
#[path = "motion_tests.rs"]
mod tests;
