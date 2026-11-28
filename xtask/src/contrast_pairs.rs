//! `check-contrast-pairs` (plan E7.7.0a): every text-on-ground pair the
//! region map declares reads at least 4.5:1 (WCAG 1.4.3), and every graphic
//! pair (`graphic = true`, the status dots) at least 3:1 (1.4.11), on each
//! base; `gen-contrast-pairs` writes that pair set for the skin validator.
//!
//! The text colour of each pair is the rule's own `color` (every rule that
//! sets it, not only the first); the ground is the entry's `on` list in
//! `src/styles/regions.toml`; both resolve against the registry tokens
//! (`src/styles/generated/tokens.css`, plan E7.7.1) and the `@layer sec`
//! tokens of `src/styles/global.css`. The measurement is
//! [`rekindle_skin::access::model`]'s, the same the validator runs on every
//! skin's variants over the committed [`MODEL_FILE`]; this check fails while
//! that file is not the pair set the sheets and the map give.
//!
//! Two passes:
//!
//! - **dark** (the default): every declared pair on the `@theme` values;
//! - **light**: every declared pair on the `@theme` values overlaid with the
//!   light base's (`:root[data-skin-base="light"]` in `@layer
//!   xfire-defaults`), grounds included: the registry gives the light base
//!   its own grounds, so no stand-in is needed.
//!
//! A value the check cannot resolve fails it: a new colour form is taught
//! here, never skipped.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
pub(crate) use rekindle_skin::access::model::{ground, mix, token, var_colour, Rgb};
use rekindle_skin::access::model::{ModelPair, MODEL_FILE};

use crate::declared_grounds::{self as grounds, Entry, TextRule, MAP};
use crate::pair_model;

/// One pair of the model and the declaration its colour comes from.
struct Row {
    site: String,
    selector: String,
    state: String,
    value: String,
    ground: String,
    alpha: f64,
    graphic: bool,
}

impl Row {
    fn pair(&self) -> ModelPair<'_> {
        ModelPair {
            selector: &self.selector,
            state: &self.state,
            value: &self.value,
            ground: &self.ground,
            alpha: self.alpha,
            graphic: self.graphic,
        }
    }
}

/// Each rule on each ground its entry declares (`kind` names the pass:
/// `text`, `icon`, `boundary`), at the opacity it is drawn at.
fn rows<'a>(
    rules: impl Iterator<Item = &'a TextRule>,
    map: &BTreeMap<String, Entry>,
    alpha: &BTreeMap<String, f64>,
    kind: &str,
) -> Vec<Row> {
    let mut out = Vec::new();
    for rule in rules {
        // An unmapped selector is `check-declared-grounds`' finding.
        let Some(entry) = map.get(&rule.selector) else {
            continue;
        };
        let states = [
            (kind.to_owned(), &entry.on),
            (format!("{kind} hovered"), &entry.on_hover),
            (format!("{kind} selected"), &entry.on_selected),
        ];
        for (state, list) in states {
            for declared in list {
                out.push(Row {
                    site: rule.site.clone(),
                    selector: rule.selector.clone(),
                    state: state.clone(),
                    value: rule.value.clone(),
                    ground: declared.clone(),
                    alpha: alpha.get(&rule.selector).copied().unwrap_or(1.0),
                    graphic: entry.graphic,
                });
            }
        }
    }
    out
}

/// What a measurement found.
#[derive(Default)]
struct Report {
    measured: usize,
    /// Pairs under their floor, and values or grounds that did not resolve.
    problems: Vec<String>,
}

fn measure_rows(rows: &[Row], tokens: &BTreeMap<String, String>) -> Report {
    let mut report = Report::default();
    for row in rows {
        let pair = row.pair();
        let floor = pair.floor();
        report.measured += 1;
        let dimmed = if row.alpha < 1.0 {
            format!(" at opacity {:.2}", row.alpha)
        } else {
            String::new()
        };
        match pair.measure(tokens) {
            Ok(m) if m.ratio >= floor => {}
            Ok(m) => report.problems.push(format!(
                "{}  `{}` {}: `{}`{dimmed} on `{}` reads {:.2}:1, needs {floor}:1",
                row.site, row.selector, row.state, row.value, row.ground, m.ratio
            )),
            Err(why) => report
                .problems
                .push(format!("{}  `{}`: {why}", row.site, row.selector)),
        }
    }
    report
}

/// The light base's tokens: the defaults with the light values over them.
fn light_tokens(
    defaults: &BTreeMap<String, String>,
    light: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut tokens = defaults.clone();
    tokens.extend(light.iter().map(|(k, v)| (k.clone(), v.clone())));
    tokens
}

/// Every skinnable pair: text, dimmed icons and boundaries (not `@layer
/// sec`, whose tokens no skin reaches).
struct Inputs {
    text: Vec<Row>,
    icons: Vec<Row>,
    bounds: Vec<Row>,
    sec_text: Vec<Row>,
    sec_bounds: Vec<Row>,
    global: String,
    structure: Vec<String>,
}

fn inputs(root: &Path) -> Result<Inputs> {
    let (sheets, global, map_text) = grounds::read_inputs(root)?;
    let rules = grounds::text_rules(&sheets);
    let map = grounds::read_map(&map_text)?;
    let model = pair_model::read_model(&map_text)?;
    let defaults = grounds::theme_colours(&global);
    let alpha = pair_model::alphas(&sheets, &map, &model, false);
    // Dimmed icons, drawn in a text selector's colour, at 3:1.
    let icon_alpha = pair_model::alphas(&sheets, &map, &model, true);
    let icons: BTreeMap<String, Entry> = map
        .iter()
        .filter(|(s, _)| icon_alpha.contains_key(*s))
        .map(|(s, e)| {
            let mut e = e.clone();
            e.graphic = true;
            (s.clone(), e)
        })
        .collect();
    let opaque = BTreeMap::new();
    let (bounds, sec_bounds) = pair_model::boundary_rules(&sheets, &model);
    let sec_rules = pair_model::sec_text_rules(&sheets);
    let sec_dark = pair_model::sec_bases(&global)
        .first()
        .map(|(_, t)| t.clone())
        .unwrap_or_default();
    let structure = pair_model::structure(&sheets, &map, &model, &defaults, &sec_dark);
    Ok(Inputs {
        text: rows(rules.iter(), &map, &alpha, "text"),
        icons: rows(rules.iter(), &icons, &icon_alpha, "icon"),
        bounds: rows(bounds.iter(), &model.boundaries, &opaque, "boundary"),
        sec_text: rows(sec_rules.iter(), &model.sec, &opaque, "text"),
        sec_bounds: rows(sec_bounds.iter(), &model.boundaries, &opaque, "boundary"),
        global,
        structure,
    })
}

/// The content of [`MODEL_FILE`]: every skinnable pair once, sorted.
fn render(i: &Inputs) -> String {
    let mut lines: Vec<String> = i
        .text
        .iter()
        .chain(&i.icons)
        .chain(&i.bounds)
        .map(|r| r.pair().row())
        .collect();
    lines.sort();
    lines.dedup();
    let mut out = String::from(
        "# Generated by `cargo xtask gen-contrast-pairs` from src/styles/regions.toml and the\n\
         # sheets: selector, state, value, ground, opacity, graphic. Do not edit.\n",
    );
    for l in lines {
        out.push_str(&l);
        out.push('\n');
    }
    out
}

/// Writes [`MODEL_FILE`].
pub(crate) fn gen_contrast_pairs(root: &Path) -> Result<()> {
    let text = render(&inputs(root)?);
    std::fs::write(root.join(MODEL_FILE), text).with_context(|| format!("writing {MODEL_FILE}"))?;
    println!("    wrote {MODEL_FILE}");
    Ok(())
}

pub(crate) fn check_contrast_pairs(root: &Path) -> Result<()> {
    let i = inputs(root)?;
    let defaults = grounds::theme_colours(&i.global);
    let light_all = light_tokens(&defaults, &grounds::light_base_colours(&i.global));
    let mut problems: Vec<String> = i.structure.iter().map(|p| format!("map    {p}")).collect();
    match std::fs::read_to_string(root.join(MODEL_FILE)) {
        Ok(on_disk) if on_disk == render(&i) => {}
        Ok(_) => problems.push(format!(
            "model  {MODEL_FILE} is stale: run `cargo xtask gen-contrast-pairs`"
        )),
        Err(e) => problems.push(format!("model  {MODEL_FILE}: {e}")),
    }
    let mut counts = Vec::new();
    let mut run = |label: String, report: Report| {
        counts.push(format!("{} {label}", report.measured));
        problems.extend(report.problems.iter().map(|p| format!("{label}  {p}")));
    };
    for (base, tokens) in [("dark", &defaults), ("light", &light_all)] {
        run(base.into(), measure_rows(&i.text, tokens));
        run(format!("{base} icon"), measure_rows(&i.icons, tokens));
        run(format!("{base} boundary"), measure_rows(&i.bounds, tokens));
    }
    for (base, tokens) in &pair_model::sec_bases(&i.global) {
        run(format!("sec {base}"), measure_rows(&i.sec_text, tokens));
        run(
            format!("sec {base} boundary"),
            measure_rows(&i.sec_bounds, tokens),
        );
    }
    if problems.is_empty() {
        println!("  pairs measured: {}", counts.join(", "));
        return Ok(());
    }
    let measured = counts.len();
    for p in &problems {
        println!("  ✗ {p}");
    }
    Err(anyhow!(
        "{} problem(s) over {measured} passes of {MAP}. Text needs 4.5:1 on \
         each declared ground (`on`, `on_hover`, `on_selected`, at its opacity), \
         a `graphic = true` entry and every [boundaries] entry 3:1, on each \
         base: change the text role or the tint by the minimal Oklab-lightness \
         step and record the value in the registry table \
         (`crates/rekindle-skin/src/registry/table.rs`, dark or light), then \
         `cargo xtask gen-skin-registry`; keep the map true to the sheets.",
        problems.len()
    ))
}

#[cfg(test)]
#[path = "contrast_pairs_tests.rs"]
mod tests;
