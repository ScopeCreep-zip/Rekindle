//! `check-declared-grounds` (plan E7.7.0): every selector that colours text
//! with a token declares the ground the text sits on.
//!
//! A text rule's ground usually comes from an ancestor (`.reply-preview-icon`
//! sits on `.reply-preview`'s tint), so it cannot be read from the CSS. The
//! region map `src/styles/regions.toml` carries it, authored: one entry per
//! selector with an `on` list naming each ground the text is drawn on. E7.7.1
//! adds each entry's region and its validator measures every declared pair on
//! every skin. This gate keeps the map complete and true to the sheet:
//!
//! - every selector of a rule whose `color` reads a themeable `--color-*`
//!   token has an entry with at least one ground;
//! - every ground is a `--color-*` token defined in `global.css`'s
//!   `@theme`, or `<token> <N>% over <ground>` for a tint (composited in
//!   gamma-encoded sRGB like `color-mix(in srgb, …)`);
//! - no entry names a selector that is not such a rule (no stale entries).
//!
//! `@layer sec` is out of scope: its tokens are fixed, never skinned.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::css;

pub(crate) const MAP: &str = "src/styles/regions.toml";
const TOKEN_FILE: &str = "src/styles/global.css";

/// The selectors whose `color` reads a themeable token, with one site each.
pub(crate) fn text_token_selectors(sheets: &[(String, String)]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (rel, src) in sheets {
        for block in css::blocks(src) {
            if !block.is_rule() || block.in_keyframes() || block.in_layer("sec") {
                continue;
            }
            let Some(decl) = block.decls.iter().find(|d| {
                d.prop == "color"
                    && css::var_refs(&d.value)
                        .iter()
                        .any(|v| v.starts_with("--color-"))
            }) else {
                continue;
            };
            for sel in block.selectors() {
                out.entry(sel)
                    .or_insert_with(|| format!("{rel}:{}", decl.line));
            }
        }
    }
    out
}

/// The `--color-*` tokens `@theme` defines.
fn theme_tokens(global: &str) -> BTreeSet<String> {
    css::blocks(global)
        .into_iter()
        .filter(|b| b.prelude == "@theme")
        .flat_map(|b| b.decls)
        .map(|d| d.prop)
        .filter(|p| p.starts_with("--color-"))
        .collect()
}

/// Why a ground is malformed, if it is.
fn ground_problem(ground: &str, tokens: &BTreeSet<String>) -> Option<String> {
    let mut rest = ground.trim();
    loop {
        let Some((tint, under)) = rest.split_once(" over ") else {
            return (!tokens.contains(rest))
                .then(|| format!("`{rest}` is not a `@theme` colour token"));
        };
        let mut words = tint.split_whitespace();
        let (Some(token), Some(pct), None) = (words.next(), words.next(), words.next()) else {
            return Some(format!("`{tint}` is not `<token> <N>%`"));
        };
        if !tokens.contains(token) {
            return Some(format!("`{token}` is not a `@theme` colour token"));
        }
        let ok_pct = pct
            .strip_suffix('%')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=100).contains(&n));
        if !ok_pct {
            return Some(format!("`{pct}` is not a percentage 1–100%"));
        }
        rest = under.trim();
    }
}

/// Entries of the map: selector → grounds.
fn read_map(text: &str) -> Result<BTreeMap<String, Vec<String>>> {
    let doc: toml_edit::DocumentMut = text.parse().context("parsing the region map")?;
    let table = doc
        .get("selectors")
        .and_then(|t| t.as_table_like())
        .ok_or_else(|| anyhow!("{MAP} has no [selectors] table"))?;
    let mut out = BTreeMap::new();
    for (selector, item) in table.iter() {
        let on = item
            .as_table_like()
            .and_then(|t| t.get("on"))
            .and_then(|o| o.as_array())
            .ok_or_else(|| anyhow!("`{selector}` has no `on = [...]`"))?;
        let grounds = on
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("`{selector}`: a ground is not a string"))
            })
            .collect::<Result<Vec<_>>>()?;
        out.insert(selector.to_string(), grounds);
    }
    Ok(out)
}

fn compare(
    wanted: &BTreeMap<String, String>,
    map: &BTreeMap<String, Vec<String>>,
    tokens: &BTreeSet<String>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (selector, site) in wanted {
        match map.get(selector) {
            None => problems.push(format!(
                "{site}  `{selector}` colours text with a token but declares no ground"
            )),
            Some(on) if on.is_empty() => {
                problems.push(format!("{MAP}  `{selector}` has an empty `on`"));
            }
            Some(_) => {}
        }
    }
    for (selector, on) in map {
        if !wanted.contains_key(selector) {
            problems.push(format!(
                "{MAP}  `{selector}` is not a selector that colours text with a token"
            ));
        }
        for ground in on {
            if let Some(why) = ground_problem(ground, tokens) {
                problems.push(format!("{MAP}  `{selector}`: {why}"));
            }
        }
    }
    problems
}

pub(crate) fn check_declared_grounds(root: &Path) -> Result<()> {
    let sheets = css::stylesheets(root)?;
    let global = sheets
        .iter()
        .find(|(rel, _)| rel == TOKEN_FILE)
        .map(|(_, src)| src.as_str())
        .ok_or_else(|| anyhow!("{TOKEN_FILE} not found"))?;
    let map_text =
        std::fs::read_to_string(root.join(MAP)).with_context(|| format!("reading {MAP}"))?;
    let problems = compare(
        &text_token_selectors(&sheets),
        &read_map(&map_text)?,
        &theme_tokens(global),
    );
    if problems.is_empty() {
        return Ok(());
    }
    for p in &problems {
        println!("  ✗ {p}");
    }
    Err(anyhow!(
        "{} declared-ground problem(s). Every selector that colours text with \
         a token names its ground(s) in {MAP} (`on = [\"--color-…\"]`, or \
         `\"<token> N% over <ground>\"` for a tint).",
        problems.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheets() -> Vec<(String, String)> {
        let css = "@layer components {\n  .a, .b:hover { color: var(--color-x-text); }\n  .c { color: inherit; background: var(--color-x-bg); }\n  @media (x) { .d::placeholder { color: var(--color-x-dim, var(--color-x-text)); } }\n}\n@layer sec { .s { color: var(--sec-fg); } }\n@keyframes k { 0% { color: var(--color-x-text); } }";
        vec![("src/styles/theme/00-x.css".into(), css.into())]
    }

    fn tokens() -> BTreeSet<String> {
        theme_tokens("@theme {\n  --color-x-text: #fff;\n  --color-x-bg: #000;\n  --color-x-tint: #00f;\n  --font-x: a;\n}")
    }

    fn map(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(s, on)| {
                (
                    (*s).to_string(),
                    on.iter().map(|g| (*g).to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn finds_the_text_token_selectors() {
        let wanted = text_token_selectors(&sheets());
        assert_eq!(
            wanted.keys().collect::<Vec<_>>(),
            vec![".a", ".b:hover", ".d::placeholder"]
        );
        assert_eq!(wanted[".a"], "src/styles/theme/00-x.css:2");
    }

    #[test]
    fn a_complete_true_map_passes() {
        let m = map(&[
            (".a", &["--color-x-bg"]),
            (".b:hover", &["--color-x-tint 30% over --color-x-bg"]),
            (
                ".d::placeholder",
                &[
                    "--color-x-bg",
                    "--color-x-tint 10% over --color-x-tint 5% over --color-x-bg",
                ],
            ),
        ]);
        let problems = compare(&text_token_selectors(&sheets()), &m, &tokens());
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn a_missing_empty_stale_or_malformed_entry_fails() {
        let wanted = text_token_selectors(&sheets());
        let missing = map(&[(".a", &["--color-x-bg"]), (".b:hover", &["--color-x-bg"])]);
        let p = compare(&wanted, &missing, &tokens());
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("`.d::placeholder` colours text"), "{p:?}");

        let full = |extra: (&'static str, &'static [&'static str])| {
            let mut m = map(&[
                (".a", &["--color-x-bg"]),
                (".b:hover", &["--color-x-bg"]),
                (".d::placeholder", &["--color-x-bg"]),
            ]);
            m.insert(
                extra.0.to_string(),
                extra.1.iter().map(|g| (*g).to_string()).collect(),
            );
            compare(&wanted, &m, &tokens())
        };
        assert!(full((".a", &[]))[0].contains("empty `on`"));
        assert!(full((".gone", &["--color-x-bg"]))[0].contains("is not a selector"));
        assert!(full((".a", &["--color-x-nope"]))[0].contains("not a `@theme` colour token"));
        assert!(full((".a", &["--font-x"]))[0].contains("not a `@theme` colour token"));
        assert!(full((".a", &["--color-x-tint 0% over --color-x-bg"]))[0].contains("percentage"));
        assert!(full((".a", &["--color-x-tint over --color-x-bg"]))[0].contains("<token> <N>%"));
    }

    #[test]
    fn the_map_format_reads() {
        let m = read_map("[selectors]\n'.a' = { on = [\"--color-x-bg\"] }\n'.b[data-x=\"y\"]' = { on = [\"--color-x-bg\", \"--color-x-tint 30% over --color-x-bg\"] }\n").unwrap();
        assert_eq!(m[".b[data-x=\"y\"]"].len(), 2);
        assert!(read_map("[selectors]\n'.a' = { region = \"x\" }\n").is_err());
    }
}
