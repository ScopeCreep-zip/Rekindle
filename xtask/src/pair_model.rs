//! What plan E7.7.0b adds to the pair model of `check-contrast-pairs`.
//!
//! - **Own grounds.** A rule that colours text and paints its own
//!   `background` draws that text on it: every ground its entry lists is
//!   that paint (an opaque colour) or begins with it (a tint over whatever
//!   is under).
//! - **State grounds.** `S:hover` (or `S:hover:not(:disabled)`) and the
//!   selected forms in [`SELECTED`] that repaint the background without
//!   recolouring the text put `S`'s text on another ground: the entry lists
//!   it in `on_hover` / `on_selected`, measured like `on`. A state set on
//!   an ancestor (a danger item inside a hovered `.context-menu-item`) is
//!   authored the same way.
//! - **Dimming.** An `opacity` under 1 composites the text over its ground.
//!   A text rule's own opacity is read from the sheets; an ancestor's is
//!   declared in `[dimmers]`, whose `dims` names the text selectors it dims.
//!   An inactive control (a `disabled` selector) is exempt, as WCAG 1.4.3
//!   exempts inactive user-interface components.
//! - **No `filter: brightness()`.** A brightness hover cannot be measured
//!   against the tokens; it is a token step instead.
//! - **Boundaries** (WCAG 1.4.11, 3:1). Every focus ring (an `outline`
//!   reading a token) and each authored control boundary has a
//!   `[boundaries]` entry naming the grounds it is drawn on.
//! - **`@layer sec`.** Its text pairs are in `[sec]`, measured on the
//!   reserved tokens of each base: dark, light, and each with
//!   `prefers-contrast: more`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, Context, Result};

use crate::contrast_pairs::{ground, mix, token, var_colour, Rgb};
use crate::css::{self, Block};
use crate::declared_grounds::{self as grounds, Entry, TextRule, MAP};

mod sec;
pub(crate) use sec::sec_bases;

const HOVER: &[&str] = &[":hover", ":hover:not(:disabled)"];
const SELECTED: &[&str] = &[
    ".active",
    ".selected",
    "[aria-selected=\"true\"]",
    "[aria-current=\"page\"]",
    "[data-active]",
    "[data-selected]",
];
const BACKGROUND: &[&str] = &["background", "background-color"];

/// A background a rule paints.
#[derive(Debug, Clone)]
pub(crate) enum Paint {
    Opaque(Rgb),
    /// A colour at N% over whatever is under the element.
    Tint(Rgb, f64),
    /// A gradient's opaque stops: the text crosses each.
    Stops(Vec<Rgb>),
}

/// The tables E7.7.0b adds to the region map.
pub(crate) struct Model {
    /// `@layer sec` text selectors → grounds (`--sec-*` tokens).
    pub sec: BTreeMap<String, Entry>,
    /// Focus rings and control boundaries → grounds; always 3:1.
    pub boundaries: BTreeMap<String, Entry>,
    /// A dimming selector → what it dims.
    pub dimmers: BTreeMap<String, Dimmer>,
}

/// One `[dimmers]` entry.
pub(crate) struct Dimmer {
    /// The text selectors it dims, or whose colour a dimmed icon is drawn in.
    pub dims: Vec<String>,
    /// It dims an icon (WCAG 1.4.11, 3:1), not the text itself.
    pub graphic: bool,
}

pub(crate) fn read_model(text: &str) -> Result<Model> {
    let mut boundaries = grounds::read_table(text, "boundaries")?;
    for entry in boundaries.values_mut() {
        entry.graphic = true;
    }
    Ok(Model {
        sec: grounds::read_table(text, "sec")?,
        boundaries,
        dimmers: read_dimmers(text)?,
    })
}

fn read_dimmers(text: &str) -> Result<BTreeMap<String, Dimmer>> {
    let doc: toml_edit::DocumentMut = text.parse().context("parsing the region map")?;
    let table = doc
        .get("dimmers")
        .and_then(|t| t.as_table_like())
        .ok_or_else(|| anyhow!("{MAP} has no [dimmers] table"))?;
    table
        .iter()
        .map(|(selector, item)| {
            let fields = item
                .as_table_like()
                .ok_or_else(|| anyhow!("`{selector}` is not a table"))?;
            let graphic = match fields.get("graphic") {
                None => false,
                Some(g) => g
                    .as_bool()
                    .ok_or_else(|| anyhow!("`{selector}`: `graphic` is not a boolean"))?,
            };
            let dims = fields
                .get("dims")
                .and_then(|d| d.as_array())
                .ok_or_else(|| anyhow!("`{selector}` has no `dims = [...]`"))?;
            let dims = dims
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| anyhow!("`{selector}`: a dimmed selector is not a string"))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((selector.to_string(), Dimmer { dims, graphic }))
        })
        .collect()
}

/// `expr N%` → (`expr`, N); `expr` → (`expr`, none).
fn pct_part(part: &str) -> Result<(String, Option<f64>), String> {
    let part = part.trim();
    match part.rsplit_once(' ') {
        Some((expr, pct)) if pct.ends_with('%') => pct
            .trim_end_matches('%')
            .parse::<f64>()
            .ok()
            .filter(|n| (0.0..=100.0).contains(n))
            .map(|n| (expr.trim().to_string(), Some(n)))
            .ok_or_else(|| format!("`{pct}` is not a percentage")),
        _ => Ok((part.to_string(), None)),
    }
}

/// The ground a `background` value paints, if it reads a colour token.
pub(crate) fn paint(
    tokens: &BTreeMap<String, String>,
    value: &str,
) -> Result<Option<Paint>, String> {
    let value = value.trim_end_matches("!important").trim();
    if !css::var_refs(value).iter().any(|r| tokens.contains_key(r)) {
        return Ok(None);
    }
    let unsupported = || {
        format!("`{value}` is not a token, a `color-mix(in srgb, …)` of two or a gradient of those")
    };
    if let Some(args) = value
        .strip_prefix("linear-gradient(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let mut stops = Vec::new();
        // The direction reads no token; every stop is opaque.
        for part in css::split_top_level(args, ',') {
            if !reads_token(tokens, &part) {
                continue;
            }
            let (expr, _) = pct_part(&part)?;
            match paint(tokens, &expr)? {
                Some(Paint::Opaque(c)) => stops.push(c),
                _ => return Err(unsupported()),
            }
        }
        return Ok(Some(Paint::Stops(stops)));
    }
    if value.starts_with("var(") {
        return var_colour(tokens, value).map(|c| Some(Paint::Opaque(c)));
    }
    let args = value
        .strip_prefix("color-mix(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(unsupported)?;
    let parts = css::split_top_level(args, ',');
    let [space, a, b] = parts.as_slice() else {
        return Err(unsupported());
    };
    if space.trim() != "in srgb" {
        return Err(unsupported());
    }
    let ((top, pa), (under, pb)) = (pct_part(a)?, pct_part(b)?);
    let pct = match (pa, pb) {
        (Some(p), _) => p,
        (None, Some(q)) => 100.0 - q,
        (None, None) => 50.0,
    };
    let top = var_colour(tokens, &top)?;
    if under == "transparent" {
        return Ok(Some(Paint::Tint(top, pct)));
    }
    Ok(Some(Paint::Opaque(mix(
        top,
        var_colour(tokens, &under)?,
        pct,
    ))))
}

fn close(a: Rgb, b: Rgb) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.6 / 255.0)
}

/// Whether a declared ground is `paint`, or begins with it for a tint.
fn begins_with(
    tokens: &BTreeMap<String, String>,
    declared: &str,
    paint: &Paint,
) -> Result<bool, String> {
    match paint {
        Paint::Opaque(rgb) => ground(tokens, declared).map(|g| close(g, *rgb)),
        Paint::Stops(stops) => ground(tokens, declared).map(|g| stops.iter().any(|s| close(g, *s))),
        Paint::Tint(rgb, pct) => {
            let parsed = grounds::parse_ground(declared)?;
            let Some((tint, p)) = parsed.tints.first() else {
                return Ok(false);
            };
            Ok(close(token(tokens, tint)?, *rgb) && (f64::from(*p) - pct).abs() < 0.5)
        }
    }
}

/// Every style rule the model reads, with its sheet: not keyframe steps,
/// not `forced-colors` (system colours, the user's own palette).
fn rules(sheets: &[(String, String)]) -> Vec<(&str, Block)> {
    sheets
        .iter()
        .flat_map(|(rel, src)| css::blocks(src).into_iter().map(move |b| (rel.as_str(), b)))
        .filter(|(_, b)| b.is_rule() && !b.in_keyframes() && !b.under("forced-colors"))
        .collect()
}

fn last_decl<'a>(block: &'a Block, props: &[&str]) -> Option<&'a css::Decl> {
    block
        .decls
        .iter()
        .rev()
        .find(|d| props.contains(&d.prop.as_str()))
}

fn colours_text(block: &Block, prefix: &str) -> bool {
    block.decls.iter().any(|d| {
        d.prop == "color"
            && css::var_refs(&d.value)
                .iter()
                .any(|v| v.starts_with(prefix))
    })
}

fn reads_token(tokens: &BTreeMap<String, String>, value: &str) -> bool {
    css::var_refs(value).iter().any(|r| tokens.contains_key(r))
}

fn is_boundary(prop: &str) -> bool {
    prop.starts_with("outline") || prop.starts_with("border")
}

fn opacity(value: &str) -> Option<f64> {
    let value = value.trim();
    match value.strip_suffix('%') {
        Some(pct) => pct.parse::<f64>().ok().map(|n| n / 100.0),
        None => value.parse().ok(),
    }
}

fn inactive(selector: &str) -> bool {
    selector.contains("disabled")
}

/// `S:hover` / `S.active` … of a mapped `S`: (`S`, its state list, field).
fn state_of<'a>(
    selector: &str,
    entries: &'a BTreeMap<String, Entry>,
) -> Option<(String, &'a [String], &'static str)> {
    let found = |suffixes: &[&str]| {
        suffixes
            .iter()
            .filter_map(|s| selector.strip_suffix(s))
            .find_map(|base| entries.get_key_value(base))
    };
    if let Some((base, e)) = found(HOVER) {
        return Some((base.clone(), &e.on_hover, "on_hover"));
    }
    found(SELECTED).map(|(base, e)| (base.clone(), e.on_selected.as_slice(), "on_selected"))
}

struct ListCheck<'a> {
    tokens: &'a BTreeMap<String, String>,
    painter: &'a str,
    site: String,
}

impl ListCheck<'_> {
    fn run(&self, base: &str, field: &str, list: &[String], paint: &Paint) -> Vec<String> {
        if list.is_empty() {
            return vec![format!(
                "{}  `{base}`'s text sits on the background `{}` paints; list it in `{field}`",
                self.site, self.painter
            )];
        }
        list.iter()
            .filter_map(|g| match begins_with(self.tokens, g, paint) {
                Ok(true) => None,
                Ok(false) => Some(format!(
                    "{}  `{base}` {field} `{g}` is not the background `{}` paints",
                    self.site, self.painter
                )),
                Err(why) => Some(format!("{}  `{base}`: {why}", self.site)),
            })
            .collect()
    }
}

/// The sheets' facts the model's tables must match.
#[derive(Default)]
struct Seen {
    /// Non-inactive selectors under an opacity, not themselves mapped.
    dimmed: BTreeSet<String>,
    /// Selectors drawing a token-coloured border or outline.
    bounded: BTreeSet<String>,
    /// Focus rings (a token `outline`), with a site.
    ringed: BTreeMap<String, String>,
    /// `@layer sec` text selectors.
    sec_text: BTreeSet<String>,
}

fn read_block(
    rel: &str,
    block: &Block,
    entries: &BTreeMap<String, Entry>,
    tokens: &BTreeMap<String, String>,
    seen: &mut Seen,
) -> Vec<String> {
    let mut problems = Vec::new();
    let in_sec = block.in_layer("sec");
    let colours = colours_text(block, if in_sec { "--sec-" } else { "--color-" });
    let selectors = block.selectors();
    if let Some(bg) = last_decl(block, BACKGROUND) {
        let site = format!("{rel}:{}", bg.line);
        match paint(tokens, &bg.value) {
            Err(why) => problems.push(format!("{site}  `{}`: {why}", block.prelude)),
            Ok(None) => {}
            Ok(Some(p)) => {
                for selector in &selectors {
                    let check = ListCheck {
                        tokens,
                        painter: selector,
                        site: site.clone(),
                    };
                    if colours {
                        if let Some(e) = entries.get(selector) {
                            problems.extend(check.run(selector, "on", &e.on, &p));
                        }
                    } else if let Some((base, list, field)) = state_of(selector, entries) {
                        problems.extend(check.run(&base, field, list, &p));
                    }
                }
            }
        }
    }
    if let Some(op) = last_decl(block, &["opacity"]) {
        match opacity(&op.value) {
            None => problems.push(format!(
                "{rel}:{}  `{}`: `opacity: {}` is not a number",
                op.line, block.prelude, op.value
            )),
            Some(a) if a > 0.0 && a < 1.0 => seen.dimmed.extend(
                selectors
                    .iter()
                    .filter(|s| !inactive(s) && !entries.contains_key(*s))
                    .cloned(),
            ),
            Some(_) => {}
        }
    }
    if let Some(f) = block
        .decls
        .iter()
        .find(|d| d.prop == "filter" && d.value.contains("brightness("))
    {
        problems.push(format!(
            "{rel}:{}  `{}`: `filter: {}` cannot be measured; paint the state with a token step",
            f.line, block.prelude, f.value
        ));
    }
    let boundary = |outline_only: bool| {
        block.decls.iter().find(|d| {
            (if outline_only {
                d.prop.starts_with("outline")
            } else {
                is_boundary(&d.prop)
            }) && reads_token(tokens, &d.value)
        })
    };
    if boundary(false).is_some() {
        seen.bounded.extend(selectors.iter().cloned());
    }
    if let Some(ring) = boundary(true) {
        for s in &selectors {
            seen.ringed
                .entry(s.clone())
                .or_insert_with(|| format!("{rel}:{}", ring.line));
        }
    }
    if in_sec && colours {
        seen.sec_text.extend(selectors.iter().cloned());
    }
    problems
}

/// Where the map's own-ground, state, dimmer, boundary and `sec` entries
/// disagree with the sheets.
pub(crate) fn structure(
    sheets: &[(String, String)],
    map: &BTreeMap<String, Entry>,
    model: &Model,
    tokens: &BTreeMap<String, String>,
    sec_tokens: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut seen = Seen::default();
    let mut problems = Vec::new();
    for (rel, block) in rules(sheets) {
        let (entries, toks) = if block.in_layer("sec") {
            (&model.sec, sec_tokens)
        } else {
            (map, tokens)
        };
        problems.extend(read_block(rel, &block, entries, toks, &mut seen));
    }
    for (s, site) in &seen.ringed {
        if !model.boundaries.contains_key(s) {
            problems.push(format!(
                "{site}  `{s}` draws a focus ring but has no [boundaries] entry"
            ));
        }
    }
    for s in model.boundaries.keys() {
        if !seen.bounded.contains(s) {
            problems.push(format!(
                "{MAP}  [boundaries] `{s}` draws no token border or outline"
            ));
        }
    }
    for s in &seen.dimmed {
        if !model.dimmers.contains_key(s) {
            problems.push(format!(
                "{MAP}  `{s}` sets an opacity under 1 but has no [dimmers] entry"
            ));
        }
    }
    for (s, Dimmer { dims, .. }) in &model.dimmers {
        if !seen.dimmed.contains(s) {
            problems.push(format!(
                "{MAP}  [dimmers] `{s}` is not an active selector with an opacity under 1"
            ));
        }
        if dims.is_empty() {
            problems.push(format!("{MAP}  [dimmers] `{s}` dims nothing"));
        }
        for d in dims.iter().filter(|d| !map.contains_key(*d)) {
            problems.push(format!(
                "{MAP}  [dimmers] `{s}` dims `{d}`, which has no [selectors] entry"
            ));
        }
    }
    for s in &seen.sec_text {
        if !model.sec.contains_key(s) {
            problems.push(format!(
                "{MAP}  `{s}` colours `@layer sec` text but has no [sec] entry"
            ));
        }
    }
    for s in model.sec.keys() {
        if !seen.sec_text.contains(s) {
            problems.push(format!(
                "{MAP}  [sec] `{s}` is not a selector that colours `@layer sec` text"
            ));
        }
    }
    problems
}

/// The opacity each mapped text selector is drawn at: its own and its
/// declared text dimmers' (`graphic = false`), multiplied; with
/// `graphic = true`, the opacity of the icons drawn in its colour.
pub(crate) fn alphas(
    sheets: &[(String, String)],
    map: &BTreeMap<String, Entry>,
    model: &Model,
    graphic: bool,
) -> BTreeMap<String, f64> {
    let mut out: BTreeMap<String, f64> = BTreeMap::new();
    for (_, block) in rules(sheets) {
        let Some(a) = last_decl(&block, &["opacity"])
            .and_then(|d| opacity(&d.value))
            .filter(|a| *a > 0.0 && *a < 1.0)
        else {
            continue;
        };
        for s in block.selectors().iter().filter(|s| !inactive(s)) {
            let own = (!graphic && map.contains_key(s)).then(|| s.clone());
            let declared = model
                .dimmers
                .get(s)
                .filter(|d| d.graphic == graphic)
                .into_iter()
                .flat_map(|d| d.dims.iter().cloned());
            for d in own.into_iter().chain(declared) {
                *out.entry(d).or_insert(1.0) *= a;
            }
        }
    }
    out
}

/// The `@layer sec` text rules.
pub(crate) fn sec_text_rules(sheets: &[(String, String)]) -> Vec<TextRule> {
    let mut out = Vec::new();
    for (rel, block) in rules(sheets) {
        if !block.in_layer("sec") {
            continue;
        }
        let Some(decl) = block.decls.iter().find(|d| {
            d.prop == "color"
                && css::var_refs(&d.value)
                    .iter()
                    .any(|v| v.starts_with("--sec-"))
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
    out
}

/// The colour of a `border`/`outline` shorthand, written last
/// (`1px solid color-mix(…)` → `color-mix(…)`).
fn shorthand_colour(value: &str) -> &str {
    ["color-mix(", "var("]
        .iter()
        .find_map(|f| value.find(f))
        .map_or(value, |at| &value[at..])
}

/// The boundary colours of the `[boundaries]` selectors: (themeable, sec).
pub(crate) fn boundary_rules(
    sheets: &[(String, String)],
    model: &Model,
) -> (Vec<TextRule>, Vec<TextRule>) {
    let (mut theme, mut sec) = (Vec::new(), Vec::new());
    for (rel, block) in rules(sheets) {
        let (out, prefix) = if block.in_layer("sec") {
            (&mut sec, "--sec-")
        } else {
            (&mut theme, "--color-")
        };
        for selector in block.selectors() {
            if !model.boundaries.contains_key(&selector) {
                continue;
            }
            for d in block.decls.iter().filter(|d| {
                is_boundary(&d.prop)
                    && css::var_refs(&d.value)
                        .iter()
                        .any(|v| v.starts_with(prefix))
            }) {
                out.push(TextRule {
                    selector: selector.clone(),
                    value: shorthand_colour(&d.value).to_string(),
                    site: format!("{rel}:{}", d.line),
                });
            }
        }
    }
    (theme, sec)
}

#[cfg(test)]
mod tests;
