//! The pair model of `src/styles/regions.toml` (plan E7.7.0a, E7.7.0b),
//! measured on any token set: the defaults (`cargo xtask
//! check-contrast-pairs`) and every skin's resolved variant (the
//! validator, [`super::failures`]). One implementation for both.
//!
//! `cargo xtask gen-contrast-pairs` reads the sheets and the map (the CSS
//! parse stays in xtask) and writes every pair the check measures, except
//! the fixed `@layer sec` ones, to [`MODEL_FILE`]: each text rule on each
//! declared ground (`on`, `on_hover`, `on_selected`) at the opacity it is
//! drawn at, each dimmed icon (3:1) and each `[boundaries]` entry (3:1).
//! The check fails while that file is stale.
//!
//! Colours are composited in gamma-encoded sRGB as `color-mix(in srgb, …)`
//! does; a text colour of `color-mix(in srgb, <token> N%, transparent)` and
//! a translucent text token are composited over each ground, and the text
//! then at its opacity. Luminance and ratio are WCAG 2.x relative
//! luminance (sRGB transfer per CSS Color 4).

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The generated pair table, relative to the workspace root.
pub const MODEL_FILE: &str = "crates/rekindle-skin/src/access/model.tsv";

/// WCAG 1.4.3 and 1.4.11.
pub const TEXT_FLOOR: f64 = 4.5;
pub const GRAPHIC_FLOOR: f64 = 3.0;

pub type Rgb = [f64; 3];

/// `#rgb`, `#rrggbb` or `#rrggbbaa` as gamma-encoded channels and alpha
/// in 0..=1.
fn hex(value: &str) -> Option<[f64; 4]> {
    let digits = value.trim().strip_prefix('#')?;
    if !digits.is_ascii() {
        return None;
    }
    let long = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => format!("{digits}ff"),
        8 => digits.to_string(),
        _ => return None,
    };
    let channel = |i: usize| {
        u8::from_str_radix(long.get(i..i + 2)?, 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    let alpha = if long.len() == 6 {
        Some(1.0)
    } else {
        channel(6)
    };
    Some([channel(0)?, channel(2)?, channel(4)?, alpha?])
}

fn rgba(tokens: &BTreeMap<String, String>, name: &str) -> Result<[f64; 4], String> {
    let value = tokens
        .get(name)
        .ok_or_else(|| format!("`{name}` is not a `@theme` colour token"))?;
    hex(value).ok_or_else(|| format!("`{name}: {value}` is not a hex colour"))
}

/// Whether `name` is a colour token of `tokens` drawn below full opacity.
#[must_use]
pub fn translucent(tokens: &BTreeMap<String, String>, name: &str) -> bool {
    rgba(tokens, name).is_ok_and(|[_, _, _, a]| a < 1.0)
}

/// An opaque token's colour; a translucent one is no ground.
pub fn token(tokens: &BTreeMap<String, String>, name: &str) -> Result<Rgb, String> {
    if translucent(tokens, name) {
        return Err(format!("`{name}` is translucent: a ground must be opaque"));
    }
    let [r, g, b, _] = rgba(tokens, name)?;
    Ok([r, g, b])
}

/// `top` at `pct`% over `under`, per channel in gamma-encoded sRGB.
#[must_use]
pub fn mix(top: Rgb, under: Rgb, pct: f64) -> Rgb {
    let a = pct / 100.0;
    [0, 1, 2].map(|i| top[i] * a + under[i] * (1.0 - a))
}

/// A token's colour composited over `under` at `pct`% of its own alpha.
fn tint_over(
    tokens: &BTreeMap<String, String>,
    name: &str,
    under: Rgb,
    pct: f64,
) -> Result<Rgb, String> {
    let [r, g, b, a] = rgba(tokens, name)?;
    Ok(mix([r, g, b], under, pct * a))
}

/// The opaque colour of a declared ground.
pub fn ground(tokens: &BTreeMap<String, String>, declared: &str) -> Result<Rgb, String> {
    let parsed = parse_ground(declared)?;
    let mut rgb = token(tokens, parsed.base)?;
    for (tint, pct) in parsed.tints.iter().rev() {
        rgb = tint_over(tokens, tint, rgb, share(tokens, pct)?)?;
    }
    Ok(rgb)
}

/// The first `var()` of `expr` that names a token of `tokens`. In
/// `var(--profile-accent, var(--color-xfire-accent-text))` the first is a
/// runtime property, so the default is its fallback.
pub fn var_name<'t>(tokens: &'t BTreeMap<String, String>, expr: &str) -> Result<&'t str, String> {
    var_refs(expr)
        .into_iter()
        .find_map(|r| tokens.get_key_value(&r).map(|(k, _)| k.as_str()))
        .ok_or_else(|| format!("`{expr}` names no `@theme` colour token"))
}

/// The opaque colour of the first token `expr` names.
pub fn var_colour(tokens: &BTreeMap<String, String>, expr: &str) -> Result<Rgb, String> {
    token(tokens, var_name(tokens, expr)?)
}

/// The colour a `color` value paints over `under`.
pub fn text_colour(
    tokens: &BTreeMap<String, String>,
    value: &str,
    under: Rgb,
) -> Result<Rgb, String> {
    let value = value.trim_end_matches("!important").trim();
    let Some(args) = value
        .strip_prefix("color-mix(")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return tint_over(tokens, var_name(tokens, value)?, under, 100.0);
    };
    let unsupported = || format!("`{value}` is not `color-mix(in srgb, <token> N%, transparent)`");
    let parts = split_top_level(args, ',');
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
    tint_over(tokens, var_name(tokens, expr)?, under, pct)
}

/// WCAG 2.x relative luminance and contrast ratio: the crate's one
/// implementation, on gamma-encoded channels in 0..=1.
pub use crate::colour::{contrast_unit as contrast, luminance_unit as luminance};

/// The prefix of the registry's tint-share tokens.
pub const TINT_SHARE: &str = "--tint-";

/// A tint's share in percent: `N%`, or `var(--tint-…)` read from `tokens`.
pub fn share(tokens: &BTreeMap<String, String>, word: &str) -> Result<f64, String> {
    let value = match share_token(word) {
        Some(name) => tokens
            .get(name)
            .ok_or_else(|| format!("`{name}` is not a registry tint share"))?
            .as_str(),
        None => word,
    };
    value
        .trim()
        .strip_suffix('%')
        .and_then(|n| n.parse::<f64>().ok())
        .filter(|n| (0.0..=100.0).contains(n))
        .ok_or_else(|| format!("`{word}` is not a percentage"))
}

/// The registry share token `var(--tint-…)` names, if any.
#[must_use]
pub fn share_token(word: &str) -> Option<&str> {
    word.strip_prefix("var(")
        .and_then(|w| w.strip_suffix(')'))
        .filter(|name| name.starts_with(TINT_SHARE))
}

/// A declared ground: its tints from the top down, over a base token.
pub struct Ground<'a> {
    /// Each tint's token and share (`N%` or `var(--tint-…)`).
    pub tints: Vec<(&'a str, &'a str)>,
    pub base: &'a str,
}

/// The grammar of a ground, without checking that its tokens exist.
pub fn parse_ground(ground: &str) -> Result<Ground<'_>, String> {
    let mut rest = ground.trim();
    let mut tints = Vec::new();
    while let Some((tint, under)) = rest.split_once(" over ") {
        let mut words = tint.split_whitespace();
        let (Some(token), Some(pct), None) = (words.next(), words.next(), words.next()) else {
            return Err(format!("`{tint}` is not `<token> <N>%`"));
        };
        // A CSS percentage may be fractional: the text-field selection is
        // declared 99.9% (00-primitives.css: WebKit swaps an opaque author
        // selection colour, blendWithWhite, and leaves a non-opaque one).
        let literal = pct
            .strip_suffix('%')
            .and_then(|n| n.parse::<f64>().ok())
            .is_some_and(|n| n > 0.0 && n <= 100.0);
        if !literal && share_token(pct).is_none() {
            return Err(format!(
                "`{pct}` is not a percentage above 0% up to 100% or `var(--tint-…)`"
            ));
        }
        tints.push((token, pct));
        rest = under.trim();
    }
    Ok(Ground { tints, base: rest })
}

/// Split at `sep` outside parentheses, brackets and strings.
#[must_use]
pub fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut cur = String::new();
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ if c == sep && depth == 0 => {
                    parts.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            },
        }
        cur.push(c);
    }
    parts.push(cur);
    parts
}

/// The custom properties a value reads: `var(--a, var(--b))` → `--a`, `--b`.
#[must_use]
pub fn var_refs(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(at) = rest.find("var(") {
        let after = rest[at + 4..].trim_start();
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if name.starts_with("--") {
            out.push(name);
        }
        rest = &rest[at + 4..];
    }
    out
}

/// One pair of the model: a colour value on a declared ground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPair<'a> {
    /// The selector whose colour it is.
    pub selector: &'a str,
    /// `text`, `hovered`, `selected`, `icon` or `boundary`.
    pub state: &'a str,
    /// The `color` (or boundary) value as written.
    pub value: &'a str,
    /// The ground as `regions.toml` declares it.
    pub ground: &'a str,
    /// The opacity the colour is drawn at (its own and its dimmers').
    pub alpha: f64,
    /// 3:1 (a graphic, an icon, a boundary), else 4.5:1.
    pub graphic: bool,
}

/// What [`ModelPair::measure`] found: the composited colour, the opaque
/// ground and their ratio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairMeasure {
    pub fg: Rgb,
    pub ground: Rgb,
    pub ratio: f64,
}

impl ModelPair<'_> {
    #[must_use]
    pub fn floor(&self) -> f64 {
        if self.graphic {
            GRAPHIC_FLOOR
        } else {
            TEXT_FLOOR
        }
    }

    /// The pair on `tokens` (CSS custom property → value).
    pub fn measure(&self, tokens: &BTreeMap<String, String>) -> Result<PairMeasure, String> {
        let under = ground(tokens, self.ground)?;
        let text = text_colour(tokens, self.value, under)?;
        let fg = mix(text, under, self.alpha * 100.0);
        Ok(PairMeasure {
            fg,
            ground: under,
            ratio: contrast(fg, under),
        })
    }

    /// One line of [`MODEL_FILE`].
    #[must_use]
    pub fn row(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{:?}\t{}",
            self.selector, self.state, self.value, self.ground, self.alpha, self.graphic
        )
    }
}

fn parse_row(line: &str) -> Option<ModelPair<'_>> {
    let mut f = line.split('\t');
    let pair = ModelPair {
        selector: f.next()?,
        state: f.next()?,
        value: f.next()?,
        ground: f.next()?,
        alpha: f.next()?.parse().ok()?,
        graphic: f.next()?.parse().ok()?,
    };
    f.next().is_none().then_some(pair)
}

/// The rows of a model file; `Err` names the first line that is not one.
pub fn parse_model(text: &str) -> Result<Vec<ModelPair<'_>>, String> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| parse_row(l).ok_or_else(|| format!("`{l}` is not a model row")))
        .collect()
}

/// The committed model ([`MODEL_FILE`]).
#[must_use]
pub fn model() -> &'static [ModelPair<'static>] {
    static MODEL: OnceLock<Vec<ModelPair<'static>>> = OnceLock::new();
    MODEL.get_or_init(|| parse_model(include_str!("model.tsv")).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_model_parses_whole() {
        let text = include_str!("model.tsv");
        let rows = parse_model(text).unwrap();
        assert!(rows.len() > 100, "{}", rows.len());
        assert_eq!(model().len(), rows.len());
    }

    #[test]
    fn a_ground_share_may_be_a_fractional_percentage_above_zero_up_to_100() {
        assert_eq!(parse_ground("--a 99.9% over --b").unwrap().tints, vec![("--a", "99.9%")]);
        assert!(parse_ground("--a 100% over --b").is_ok());
        for bad in ["--a 0% over --b", "--a 100.1% over --b", "--a -5% over --b", "--a NaN% over --b"] {
            assert!(parse_ground(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_translucent_text_token_composites_and_a_translucent_ground_refuses() {
        let tokens: BTreeMap<String, String> = [
            ("--w".to_string(), "#ffffff80".to_string()),
            ("--k".to_string(), "#000000".to_string()),
        ]
        .into_iter()
        .collect();
        let pair = ModelPair {
            selector: ".a",
            state: "text",
            value: "var(--w)",
            ground: "--k",
            alpha: 1.0,
            graphic: false,
        };
        let m = pair.measure(&tokens).unwrap();
        assert!((m.fg[0] - 128.0 / 255.0).abs() < 1e-9);
        let on_glass = ModelPair {
            ground: "--w",
            ..pair
        };
        assert!(on_glass
            .measure(&tokens)
            .unwrap_err()
            .contains("translucent"));
    }
}
