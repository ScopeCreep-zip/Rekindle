//! `check-contrast-pairs` (plan E7.7.0a): every text-on-ground pair the
//! region map declares reads at least 4.5:1 (WCAG 1.4.3), and every graphic
//! pair (`graphic = true`, the status dots) at least 3:1 (1.4.11), on each
//! base.
//!
//! The text colour of each pair is the rule's own `color` (every rule that
//! sets it, not only the first); the ground is the entry's `on` list in
//! `src/styles/regions.toml`; both resolve against the tokens of
//! `src/styles/global.css`. A tint is composited in gamma-encoded sRGB as
//! `color-mix(in srgb, …)` does; a text colour of
//! `color-mix(in srgb, <token> N%, transparent)` is composited over each
//! ground. Luminance and ratio are WCAG 2.x relative luminance (sRGB
//! transfer per CSS Color 4).
//!
//! Two passes:
//!
//! - **dark** (the default): every declared pair on the `@theme` values;
//! - **light**: the pairs whose text reads a token the light base revalues
//!   (`:root[data-skin-base="light"]` in `@layer xfire-defaults`), on the
//!   `@theme` values overlaid with the light ones. The light base has no
//!   grounds of its own until E7.7.1's registry gives it light defaults,
//!   so every `--color-xfire-bg*` ground it does not revalue reads
//!   [`LIGHT_GROUND`], the white its values are calibrated on; a pair whose
//!   text the light base does not revalue is left to that registry, and
//!   counted.
//!
//! E7.7.1's validator then measures every pair on every skin and variant;
//! this check is that measurement for the built-in one. A value the check
//! cannot resolve fails it: a new colour form is taught here, never skipped.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::css;
use crate::declared_grounds::{self as grounds, Entry, TextRule, MAP};

const TEXT_FLOOR: f64 = 4.5;
/// The ground of the light base until E7.7.1's registry defines its own.
const LIGHT_GROUND: &str = "#ffffff";
/// The ground tokens [`LIGHT_GROUND`] stands in for.
const GROUND_PREFIX: &str = "--color-xfire-bg";
const GRAPHIC_FLOOR: f64 = 3.0;

type Rgb = [f64; 3];

/// `#rgb` or `#rrggbb` as gamma-encoded channels in 0..=1.
fn hex(value: &str) -> Option<Rgb> {
    let digits = value.trim().strip_prefix('#')?;
    if !digits.is_ascii() {
        return None;
    }
    let long = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_string(),
        _ => return None,
    };
    let channel = |i: usize| {
        u8::from_str_radix(long.get(i..i + 2)?, 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn token(tokens: &BTreeMap<String, String>, name: &str) -> Result<Rgb, String> {
    let value = tokens
        .get(name)
        .ok_or_else(|| format!("`{name}` is not a `@theme` colour token"))?;
    hex(value).ok_or_else(|| format!("`{name}: {value}` is not a hex colour"))
}

/// `top` at `pct`% over `under`, per channel in gamma-encoded sRGB.
fn mix(top: Rgb, under: Rgb, pct: f64) -> Rgb {
    let a = pct / 100.0;
    [0, 1, 2].map(|i| top[i] * a + under[i] * (1.0 - a))
}

fn ground(tokens: &BTreeMap<String, String>, declared: &str) -> Result<Rgb, String> {
    let parsed = grounds::parse_ground(declared)?;
    let mut rgb = token(tokens, parsed.base)?;
    for (tint, pct) in parsed.tints.iter().rev() {
        rgb = mix(token(tokens, tint)?, rgb, f64::from(*pct));
    }
    Ok(rgb)
}

/// The first `var()` of `expr` that names a `@theme` token. In
/// `var(--profile-accent, var(--color-xfire-accent-text))` the first is a
/// runtime property, so the default is its fallback.
fn var_colour(tokens: &BTreeMap<String, String>, expr: &str) -> Result<Rgb, String> {
    let name = css::var_refs(expr)
        .into_iter()
        .find(|r| tokens.contains_key(r))
        .ok_or_else(|| format!("`{expr}` names no `@theme` colour token"))?;
    token(tokens, &name)
}

/// The colour a `color` value paints over `under`.
fn text_colour(tokens: &BTreeMap<String, String>, value: &str, under: Rgb) -> Result<Rgb, String> {
    let value = value.trim_end_matches("!important").trim();
    let Some(args) = value
        .strip_prefix("color-mix(")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return var_colour(tokens, value);
    };
    let unsupported = || format!("`{value}` is not `color-mix(in srgb, <token> N%, transparent)`");
    let parts = css::split_top_level(args, ',');
    let [space, colour, rest] = parts.as_slice() else {
        return Err(unsupported());
    };
    if space.trim() != "in srgb" || rest.trim() != "transparent" {
        return Err(unsupported());
    }
    let (expr, pct) = colour.trim().rsplit_once(' ').ok_or_else(unsupported)?;
    let pct = pct
        .strip_suffix('%')
        .and_then(|n| n.parse::<f64>().ok())
        .filter(|n| (0.0..=100.0).contains(n))
        .ok_or_else(unsupported)?;
    Ok(mix(var_colour(tokens, expr)?, under, pct))
}

/// WCAG 2.x relative luminance.
fn luminance(rgb: Rgb) -> f64 {
    let linear = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// What a measurement found.
#[derive(Default)]
struct Report {
    measured: usize,
    /// Pairs under their floor, and values or grounds that did not resolve.
    problems: Vec<String>,
}

fn measure<'a>(
    rules: impl Iterator<Item = &'a TextRule>,
    map: &BTreeMap<String, Entry>,
    tokens: &BTreeMap<String, String>,
) -> Report {
    let mut report = Report::default();
    for rule in rules {
        // An unmapped selector is `check-declared-grounds`' finding.
        let Some(entry) = map.get(&rule.selector) else {
            continue;
        };
        let floor = if entry.graphic {
            GRAPHIC_FLOOR
        } else {
            TEXT_FLOOR
        };
        for declared in &entry.on {
            let pair = ground(tokens, declared)
                .and_then(|g| text_colour(tokens, &rule.value, g).map(|t| contrast(t, g)));
            report.measured += 1;
            match pair {
                Ok(ratio) if ratio >= floor => {}
                Ok(ratio) => report.problems.push(format!(
                    "{}  `{}`: `{}` on `{declared}` reads {ratio:.2}:1, needs {floor}:1",
                    rule.site, rule.selector, rule.value
                )),
                Err(why) => report
                    .problems
                    .push(format!("{}  `{}`: {why}", rule.site, rule.selector)),
            }
        }
    }
    report
}

/// The light base's tokens: the defaults, the light values over them, and
/// [`LIGHT_GROUND`] for every ground token the light base leaves unset.
fn light_tokens(
    defaults: &BTreeMap<String, String>,
    light: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut tokens = defaults.clone();
    for (name, value) in &mut tokens {
        if name.starts_with(GROUND_PREFIX) {
            LIGHT_GROUND.clone_into(value);
        }
    }
    tokens.extend(light.iter().map(|(k, v)| (k.clone(), v.clone())));
    tokens
}

/// The rules whose text reads a token the light base revalues.
fn light_rules<'a>(rules: &'a [TextRule], light: &BTreeMap<String, String>) -> Vec<&'a TextRule> {
    rules
        .iter()
        .filter(|r| {
            css::var_refs(&r.value)
                .iter()
                .any(|v| light.contains_key(v))
        })
        .collect()
}

pub(crate) fn check_contrast_pairs(root: &Path) -> Result<()> {
    let (sheets, global, map_text) = grounds::read_inputs(root)?;
    let rules = grounds::text_rules(&sheets);
    let map = grounds::read_map(&map_text)?;
    let defaults = grounds::theme_colours(&global);
    let light = grounds::light_base_colours(&global);
    let dark = measure(rules.iter(), &map, &defaults);
    let lit = light_rules(&rules, &light);
    let light_report = measure(lit.iter().copied(), &map, &light_tokens(&defaults, &light));
    let problems: Vec<String> = dark
        .problems
        .iter()
        .map(|p| format!("dark   {p}"))
        .chain(light_report.problems.iter().map(|p| format!("light  {p}")))
        .collect();
    let measured = dark.measured + light_report.measured;
    if problems.is_empty() {
        println!(
            "  {} declared pairs measured on the dark base, {} on the light base \
             ({} rules there have no light-base text value until E7.7.1)",
            dark.measured,
            light_report.measured,
            rules.len() - lit.len()
        );
        return Ok(());
    }
    for p in &problems {
        println!("  ✗ {p}");
    }
    Err(anyhow!(
        "{} of {measured} declared pair(s) in {MAP} fail. Text needs 4.5:1 on \
         each declared ground and a `graphic = true` entry 3:1, on each base: \
         change the text role or the tint by the minimal Oklab-lightness step \
         and record the value in the `@theme` defaults (dark) or the \
         `{}` block (light).",
        problems.len(),
        grounds::LIGHT_BASE
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let report = measure(rules(css).iter(), &map, &tokens());
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
        let report = measure(rules(css).iter(), &map, &tokens());
        assert!(
            report.problems[0].contains("not a hex colour"),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn the_light_pass_measures_revalued_text_on_the_light_ground() {
        let global = "@theme {\n  --color-xfire-bg-panel: #1e1e1e;\n  --color-xfire-away: #ffcc00;\n  --color-xfire-away-text: #ffcc00;\n  --color-xfire-text: #e7e7e7;\n}\n@layer xfire-defaults {\n  :root[data-skin-base=\"light\"] {\n    --color-xfire-away: #b59000;\n    --color-xfire-away-text: #8a6d00;\n  }\n}";
        let defaults = grounds::theme_colours(global);
        let light = grounds::light_base_colours(global);
        assert_eq!(light.len(), 2);
        let tokens = light_tokens(&defaults, &light);
        assert_eq!(tokens["--color-xfire-bg-panel"], LIGHT_GROUND);
        assert_eq!(tokens["--color-xfire-away"], "#b59000");
        let css = ".warn { color: var(--color-xfire-away-text); }\n.dot { color: var(--color-xfire-away); }\n.body { color: var(--color-xfire-text); }";
        let all = rules(css);
        let lit = light_rules(&all, &light);
        assert_eq!(lit.len(), 2, "the body text has no light value");
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
        let report = measure(lit.into_iter(), &map, &tokens);
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
