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
mod tests {
    use super::*;
    use rekindle_skin::access::model::{contrast, text_colour};

    fn tokens() -> BTreeMap<String, String> {
        grounds::theme_colours(
            "@theme {\n  --color-w: #ffffff;\n  --color-k: #000;\n  --color-grey: #767676;\n  --color-red: #ef4444;\n  --color-panel: #1e1e1e;\n  --color-bad: rgb(1 2 3);\n}",
        )
    }

    fn rules(css: &str) -> Vec<TextRule> {
        grounds::text_rules(&[("src/styles/theme/00-x.css".into(), css.into())])
    }

    fn entry(on: &[&str], graphic: bool) -> Entry {
        Entry {
            on: on.iter().map(|g| (*g).to_string()).collect(),
            graphic,
            ..Entry::default()
        }
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.005
    }

    #[test]
    fn ratios_match_the_wcag_vectors() {
        let t = tokens();
        assert!(approx(
            contrast(
                token(&t, "--color-w").unwrap(),
                token(&t, "--color-k").unwrap()
            ),
            21.0
        ));
        assert!(approx(
            contrast(
                token(&t, "--color-grey").unwrap(),
                token(&t, "--color-w").unwrap()
            ),
            4.54
        ));
        assert!(approx(
            contrast(
                token(&t, "--color-red").unwrap(),
                token(&t, "--color-panel").unwrap()
            ),
            4.43
        ));
        assert!(token(&t, "--color-bad")
            .unwrap_err()
            .contains("not a hex colour"));
    }

    #[test]
    fn grounds_composite_top_down() {
        let t = tokens();
        let half = ground(&t, "--color-w 50% over --color-k").unwrap();
        assert!(half.iter().all(|c| approx(*c, 0.5)));
        // 50% white over (50% white over black) = 75% white.
        let three_quarters = ground(&t, "--color-w 50% over --color-w 50% over --color-k").unwrap();
        assert!(three_quarters.iter().all(|c| approx(*c, 0.75)));
        assert!(ground(&t, "--color-nope").is_err());
    }

    #[test]
    fn text_values_resolve() {
        let t = tokens();
        let k = token(&t, "--color-k").unwrap();
        let white = |rgb: Rgb| rgb.iter().all(|c| approx(*c, 1.0));
        assert!(white(
            text_colour(&t, "var(--color-w) !important", k).unwrap()
        ));
        assert!(white(
            text_colour(&t, "var(--profile-accent, var(--color-w))", k).unwrap()
        ));
        let faded =
            text_colour(&t, "color-mix(in srgb, var(--color-w) 60%, transparent)", k).unwrap();
        assert!(faded.iter().all(|c| approx(*c, 0.6)));
        assert!(text_colour(&t, "color-mix(in srgb, var(--color-w) 60%, black)", k).is_err());
        assert!(text_colour(&t, "var(--profile-accent)", k).is_err());
    }

    #[test]
    fn a_pair_under_its_floor_fails_and_a_graphic_needs_three() {
        let css = "@layer components {\n  .ok { color: var(--color-w); }\n  .low { color: var(--color-red); }\n  .dot { color: var(--color-red); }\n}";
        let map: BTreeMap<String, Entry> = [
            (
                ".ok".to_string(),
                entry(&["--color-k", "--color-red 30% over --color-k"], false),
            ),
            (
                ".low".to_string(),
                entry(&["--color-k", "--color-panel"], false),
            ),
            (".dot".to_string(), entry(&["--color-panel"], true)),
        ]
        .into_iter()
        .collect();
        let report = measure_rows(
            &rows(rules(css).iter(), &map, &BTreeMap::new(), "text"),
            &tokens(),
        );
        assert_eq!(report.measured, 5);
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert!(report.problems[0].contains("`.low`"));
        assert!(report.problems[0].contains("on `--color-panel` reads 4.43:1, needs 4.5:1"));
    }

    #[test]
    fn an_unresolvable_ground_fails() {
        let css = ".a { color: var(--color-w); }";
        let map = [(".a".to_string(), entry(&["--color-bad"], false))]
            .into_iter()
            .collect();
        let report = measure_rows(
            &rows(rules(css).iter(), &map, &BTreeMap::new(), "text"),
            &tokens(),
        );
        assert!(
            report.problems[0].contains("not a hex colour"),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn the_light_pass_measures_every_rule_on_the_light_grounds() {
        let global = "@theme {\n  --color-xfire-bg-panel: #1e1e1e;\n  --color-xfire-away: #ffcc00;\n  --color-xfire-away-text: #ffcc00;\n  --color-xfire-text: #e7e7e7;\n}\n@layer xfire-defaults {\n  :root[data-skin-base=\"light\"] {\n    --color-xfire-bg-panel: #ffffff;\n    --color-xfire-away: #b59000;\n    --color-xfire-away-text: #8a6d00;\n  }\n}";
        let defaults = grounds::theme_colours(global);
        let light = grounds::light_base_colours(global);
        assert_eq!(light.len(), 3);
        let tokens = light_tokens(&defaults, &light);
        assert_eq!(tokens["--color-xfire-bg-panel"], "#ffffff");
        assert_eq!(tokens["--color-xfire-away"], "#b59000");
        let css = ".warn { color: var(--color-xfire-away-text); }\n.dot { color: var(--color-xfire-away); }\n.body { color: var(--color-xfire-text); }";
        let all = rules(css);
        let on = &[
            "--color-xfire-bg-panel",
            "--color-xfire-away 20% over --color-xfire-bg-panel",
        ];
        let map: BTreeMap<String, Entry> = [
            (".warn".to_string(), entry(on, false)),
            (".dot".to_string(), entry(&["--color-xfire-bg-panel"], true)),
        ]
        .into_iter()
        .collect();
        let report = measure_rows(&rows(all.iter(), &map, &BTreeMap::new(), "text"), &tokens);
        assert_eq!(report.measured, 3);
        // #8a6d00 on white is 4.96:1, on the light away tint under 4.5:1.
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert!(
            report.problems[0].contains("20% over"),
            "{:?}",
            report.problems
        );
    }

    /// The Verify of plan E7.7.0a: the shipped map and palette pass.
    #[test]
    fn the_default_palette_passes_every_declared_pair() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits in the workspace root");
        check_contrast_pairs(root).unwrap();
    }
}
