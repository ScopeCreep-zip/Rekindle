//! `check-declared-grounds` (plan E7.7.0): every selector that colours text
//! with a token declares the ground the text sits on.
//!
//! A text rule's ground usually comes from an ancestor (`.reply-preview-icon`
//! sits on `.reply-preview`'s tint), so it cannot be read from the CSS. The
//! region map `src/styles/regions.toml` carries it, authored: one entry per
//! selector with an `on` list naming each ground the text is drawn on, and
//! `graphic = true` where the colour paints a graphic rather than text.
//! E7.7.1 adds each entry's region and its validator measures every declared
//! pair on every skin; until then `check-contrast-pairs` measures them on the
//! default palette. This gate keeps the map complete and true to the sheet:
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
pub(crate) use rekindle_skin::access::model::{parse_ground, share, share_token, TINT_SHARE};

pub(crate) const MAP: &str = "src/styles/regions.toml";
pub(crate) const TOKEN_FILE: &str = "src/styles/global.css";
/// The registry tokens `global.css` imports (`cargo xtask gen-skin-registry`).
pub(crate) const GENERATED_TOKENS: &str = "src/styles/generated/tokens.css";

/// One rule's text colour, as written, for one of its selectors.
pub(crate) struct TextRule {
    pub selector: String,
    /// The `color` value (`var(--color-xfire-text)`, a `color-mix(…)`).
    pub value: String,
    /// `path:line` of the declaration.
    pub site: String,
}

/// Every (selector, `color`) of a rule whose `color` reads a themeable token.
pub(crate) fn text_rules(sheets: &[(String, String)]) -> Vec<TextRule> {
    let mut out = Vec::new();
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
            for selector in block.selectors() {
                out.push(TextRule {
                    selector,
                    value: decl.value.clone(),
                    site: format!("{rel}:{}", decl.line),
                });
            }
        }
    }
    out
}

/// The selectors whose `color` reads a themeable token, with one site each.
pub(crate) fn text_token_selectors(sheets: &[(String, String)]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for rule in text_rules(sheets) {
        out.entry(rule.selector).or_insert(rule.site);
    }
    out
}

/// A colour token, or a tint share (`--tint-*`, a registry percentage a
/// ground's tint may read).
fn is_pair_token(prop: &str) -> bool {
    prop.starts_with("--color-") || prop.starts_with(TINT_SHARE)
}

/// The `--color-*` tokens `@theme` defines, with their default values,
/// and the `--tint-*` shares.
pub(crate) fn theme_colours(global: &str) -> BTreeMap<String, String> {
    css::blocks(global)
        .into_iter()
        .filter(|b| b.prelude == "@theme")
        .flat_map(|b| b.decls)
        .filter(|d| is_pair_token(&d.prop))
        .map(|d| (d.prop, d.value))
        .collect()
}

/// The light-base selector of `@layer xfire-defaults` (E7.7.2's apply step
/// sets the attribute).
pub(crate) const LIGHT_BASE: &str = ":root[data-skin-base=\"light\"]";

/// The `--color-*` values the light base declares over the defaults.
pub(crate) fn light_base_colours(global: &str) -> BTreeMap<String, String> {
    css::blocks(global)
        .into_iter()
        .filter(|b| b.prelude == LIGHT_BASE && b.in_layer("xfire-defaults"))
        .flat_map(|b| b.decls)
        .filter(|d| is_pair_token(&d.prop))
        .map(|d| (d.prop, d.value))
        .collect()
}

fn theme_tokens(global: &str) -> BTreeSet<String> {
    theme_colours(global).into_keys().collect()
}

/// Why a ground is malformed, if it is.
fn ground_problem(ground: &str, tokens: &BTreeSet<String>) -> Option<String> {
    let parsed = match parse_ground(ground) {
        Ok(parsed) => parsed,
        Err(why) => return Some(why),
    };
    parsed
        .tints
        .iter()
        .flat_map(|(token, pct)| std::iter::once(*token).chain(share_token(pct)))
        .chain(std::iter::once(parsed.base))
        .find(|token| !tokens.contains(*token))
        .map(|token| format!("`{token}` is not a `@theme` colour token"))
}

/// One entry of the map.
#[derive(Default, Clone)]
pub(crate) struct Entry {
    /// The grounds the colour is drawn on.
    pub on: Vec<String>,
    /// The grounds it is drawn on while hovered (plan E7.7.0b).
    pub on_hover: Vec<String>,
    /// The grounds it is drawn on while selected or active (plan E7.7.0b).
    pub on_selected: Vec<String>,
    /// The colour paints a graphic (WCAG 1.4.11, 3:1), not text (1.4.3).
    pub graphic: bool,
}

/// Entries of the map's `[selectors]`: selector → entry.
pub(crate) fn read_map(text: &str) -> Result<BTreeMap<String, Entry>> {
    read_table(text, "selectors")
}

/// A list of grounds, absent meaning none.
fn ground_list(
    fields: &dyn toml_edit::TableLike,
    key: &str,
    selector: &str,
) -> Result<Option<Vec<String>>> {
    let Some(item) = fields.get(key) else {
        return Ok(None);
    };
    let list = item
        .as_array()
        .ok_or_else(|| anyhow!("`{selector}`: `{key}` is not a list"))?;
    list.iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("`{selector}`: a ground is not a string"))
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

/// Entries of one table of the map (`selectors`, `sec`, `boundaries`).
pub(crate) fn read_table(text: &str, name: &str) -> Result<BTreeMap<String, Entry>> {
    let doc: toml_edit::DocumentMut = text.parse().context("parsing the region map")?;
    let table = doc
        .get(name)
        .and_then(|t| t.as_table_like())
        .ok_or_else(|| anyhow!("{MAP} has no [{name}] table"))?;
    let mut out = BTreeMap::new();
    for (selector, item) in table.iter() {
        let fields = item
            .as_table_like()
            .ok_or_else(|| anyhow!("`{selector}` is not a table"))?;
        let on = ground_list(fields, "on", selector)?
            .ok_or_else(|| anyhow!("`{selector}` has no `on = [...]`"))?;
        let on_hover = ground_list(fields, "on_hover", selector)?.unwrap_or_default();
        let on_selected = ground_list(fields, "on_selected", selector)?.unwrap_or_default();
        let graphic = match fields.get("graphic") {
            None => false,
            Some(g) => g
                .as_bool()
                .ok_or_else(|| anyhow!("`{selector}`: `graphic` is not a boolean"))?,
        };
        out.insert(
            selector.to_string(),
            Entry {
                on,
                on_hover,
                on_selected,
                graphic,
            },
        );
    }
    Ok(out)
}

fn compare(
    wanted: &BTreeMap<String, String>,
    map: &BTreeMap<String, Entry>,
    tokens: &BTreeSet<String>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (selector, site) in wanted {
        match map.get(selector) {
            None => problems.push(format!(
                "{site}  `{selector}` colours text with a token but declares no ground"
            )),
            Some(entry) if entry.on.is_empty() => {
                problems.push(format!("{MAP}  `{selector}` has an empty `on`"));
            }
            Some(_) => {}
        }
    }
    for (selector, entry) in map {
        if !wanted.contains_key(selector) {
            problems.push(format!(
                "{MAP}  `{selector}` is not a selector that colours text with a token"
            ));
        }
        for ground in entry
            .on
            .iter()
            .chain(&entry.on_hover)
            .chain(&entry.on_selected)
        {
            if let Some(why) = ground_problem(ground, tokens) {
                problems.push(format!("{MAP}  `{selector}`: {why}"));
            }
        }
    }
    problems
}

/// The stylesheets, the token source (the generated registry tokens, then
/// `global.css` with its `@layer sec` tokens) and the map's text.
pub(crate) fn read_inputs(root: &Path) -> Result<(Vec<(String, String)>, String, String)> {
    let sheets = css::stylesheets(root)?;
    let find = |file: &str| {
        sheets
            .iter()
            .find(|(rel, _)| rel == file)
            .map(|(_, src)| src.clone())
            .ok_or_else(|| anyhow!("{file} not found"))
    };
    let global = format!("{}\n{}", find(GENERATED_TOKENS)?, find(TOKEN_FILE)?);
    let map_text =
        std::fs::read_to_string(root.join(MAP)).with_context(|| format!("reading {MAP}"))?;
    Ok((sheets, global, map_text))
}

pub(crate) fn check_declared_grounds(root: &Path) -> Result<()> {
    let (sheets, global, map_text) = read_inputs(root)?;
    let problems = compare(
        &text_token_selectors(&sheets),
        &read_map(&map_text)?,
        &theme_tokens(&global),
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

    fn map(entries: &[(&str, &[&str])]) -> BTreeMap<String, Entry> {
        entries
            .iter()
            .map(|(s, on)| {
                (
                    (*s).to_string(),
                    Entry {
                        on: on.iter().map(|g| (*g).to_string()).collect(),
                        ..Entry::default()
                    },
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
        let rules = text_rules(&sheets());
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[2].value, "var(--color-x-dim, var(--color-x-text))");
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
            m.extend(map(&[extra]));
            compare(&wanted, &m, &tokens())
        };
        assert!(full((".a", &[]))[0].contains("empty `on`"));
        assert!(full((".gone", &["--color-x-bg"]))[0].contains("is not a selector"));
        assert!(full((".a", &["--color-x-nope"]))[0].contains("not a `@theme` colour token"));
        assert!(full((".a", &["--font-x"]))[0].contains("not a `@theme` colour token"));
        assert!(full((".a", &["--color-x-nope 5% over --color-x-bg"]))[0]
            .contains("`--color-x-nope` is not"));
        assert!(full((".a", &["--color-x-tint 0% over --color-x-bg"]))[0].contains("percentage"));
        assert!(full((".a", &["--color-x-tint over --color-x-bg"]))[0].contains("<token> <N>%"));
    }

    #[test]
    fn a_ground_parses_top_down() {
        let g = parse_ground("--a 10% over --b 5% over --c").unwrap();
        assert_eq!(g.tints, vec![("--a", "10%"), ("--b", "5%")]);
        assert_eq!(g.base, "--c");
        assert!(parse_ground("--a 101% over --c").is_err());
        let g = parse_ground("--a var(--tint-fill) over --c").unwrap();
        assert_eq!(g.tints, vec![("--a", "var(--tint-fill)")]);
        assert!(parse_ground("--a var(--color-a) over --c").is_err());
    }

    #[test]
    fn the_map_format_reads() {
        let m = read_map("[selectors]\n'.a' = { on = [\"--color-x-bg\"] }\n'.b[data-x=\"y\"]' = { on = [\"--color-x-bg\", \"--color-x-tint 30% over --color-x-bg\"], graphic = true }\n").unwrap();
        assert_eq!(m[".b[data-x=\"y\"]"].on.len(), 2);
        assert!(m[".b[data-x=\"y\"]"].graphic && !m[".a"].graphic);
        assert!(read_map("[selectors]\n'.a' = { region = \"x\" }\n").is_err());
        assert!(read_map("[selectors]\n'.a' = { on = [], graphic = \"yes\" }\n").is_err());
    }
}
