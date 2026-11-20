//! The minimal Oklab-lightness walk (plan E7.7.3: "a built-in that fails
//! the validator is adjusted by the editor's minimal Oklab-lightness walk
//! per failing pair, every change recorded"; there is no exception for our
//! own skin).
//!
//! The validator is the oracle: the package is rebuilt and run through
//! [`validate::run`]; on a contrast refusal the colour the skin may set
//! (the text token, or its ground when the text is a fixed role) keeps
//! its Oklab a and b and moves its lightness away from the other colour,
//! the smallest move (bisected to 2⁻²⁰ of the way) after which the
//! validator no longer names that pair. Repeated until the package passes.
//! Oklab: Ottosson, "A perceptual color space for image processing"
//! (bottosson.github.io/posts/oklab), linear sRGB matrices.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinRule, TokenValue};

use super::ImportError;
use crate::access::scaled;
use crate::colour;
use crate::output::Canonical;
use crate::registry::{entry_in, resolve, Base, Class, TokenType, ENTRIES};
use crate::validate;

/// One change the walk made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The token whose colour changed.
    pub token: String,
    /// The validator rule and the pair it measured.
    pub rule: String,
    pub fg: String,
    pub ground: String,
    pub original: [u8; 3],
    pub new: [u8; 3],
    pub before_x100: u32,
    /// Re-measured on the result when the ground is a plain token.
    pub after_x100: Option<u32>,
    pub required_x100: u32,
}

const MAX_CHANGES: usize = 64;
const STEPS: u32 = 20;

fn to_gamma(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    u8::try_from(scaled(v * 255.0 + 0.5, 1.0)).unwrap_or(u8::MAX)
}

type Matrix = [[f64; 3]; 3];

const TO_LMS: Matrix = [
    [0.412_221_470_8, 0.536_332_536_3, 0.051_445_992_9],
    [0.211_903_498_2, 0.680_699_545_1, 0.107_396_956_6],
    [0.088_302_461_9, 0.281_718_837_6, 0.629_978_700_5],
];
const TO_OKLAB: Matrix = [
    [0.210_454_255_3, 0.793_617_785, -0.004_072_046_8],
    [1.977_998_495_1, -2.428_592_205, 0.450_593_709_9],
    [0.025_904_037_1, 0.782_771_766_2, -0.808_675_766],
];
const FROM_OKLAB: Matrix = [
    [1.0, 0.396_337_777_4, 0.215_803_757_3],
    [1.0, -0.105_561_345_8, -0.063_854_172_8],
    [1.0, -0.089_484_177_5, -1.291_485_548],
];
const FROM_LMS: Matrix = [
    [4.076_741_662_1, -3.307_711_591_3, 0.230_969_929_2],
    [-1.268_438_004_6, 2.609_757_401_1, -0.341_319_396_5],
    [-0.004_196_086_3, -0.703_418_614_7, 1.707_614_701],
];

fn mat(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

/// Gamma-encoded sRGB to Oklab `[L, a, b]`.
#[must_use]
pub fn oklab(rgb: [u8; 3]) -> [f64; 3] {
    let lin = rgb.map(crate::colour::linear);
    let lms = mat(&TO_LMS, lin).map(f64::cbrt);
    mat(&TO_OKLAB, lms)
}

/// Oklab to gamma-encoded sRGB, clamped to the gamut per channel.
#[must_use]
pub fn from_oklab(lab: [f64; 3]) -> [u8; 3] {
    let lms = mat(&FROM_OKLAB, lab).map(|v| v.powi(3));
    mat(&FROM_LMS, lms).map(to_gamma)
}

/// The smallest lightness move of `original` (towards white when `up`)
/// that `passes`, or `None` when even the end does not.
///
/// # Errors
/// What `passes` returns.
pub fn lightness_walk(
    original: [u8; 3],
    up: bool,
    mut passes: impl FnMut([u8; 3]) -> Result<bool, ImportError>,
) -> Result<Option<[u8; 3]>, ImportError> {
    let [l, a, b] = oklab(original);
    let end = if up { 1.0 } else { 0.0 };
    let at = |t: f64| from_oklab([l + (end - l) * t, a, b]);
    if !passes(at(1.0))? {
        return Ok(None);
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..STEPS {
        let mid = f64::midpoint(lo, hi);
        if passes(at(mid))? {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Ok(Some(at(hi)))
}

fn rgb_of(v: &TokenValue) -> Option<[u8; 3]> {
    match v {
        TokenValue::Color { r, g, b, .. } => Some([*r, *g, *b]),
        _ => None,
    }
}

fn colour_of(tokens: &BTreeMap<String, TokenValue>, base: Base, id: &str) -> Option<[u8; 3]> {
    resolve(ENTRIES, base, tokens)
        .into_iter()
        .find(|(e, _)| e.id == id)
        .and_then(|(_, v)| rgb_of(&v))
}

fn settable(id: &str) -> bool {
    entry_in(ENTRIES, id).is_some_and(|e| e.class == Class::Themeable && e.ty == TokenType::Color)
}

/// The pair a refusal names: (rule, fg, ground, measured, required).
fn pair_of(rule: &SkinRule) -> Option<(String, String, String, u32, u32)> {
    match rule {
        SkinRule::Contrast {
            rule,
            token,
            ground,
            measured_x100,
            required_x100,
        } => Some((
            rule.clone(),
            token.clone(),
            ground.clone(),
            *measured_x100,
            *required_x100,
        )),
        SkinRule::TextOverBitmap {
            region,
            token,
            measured_x100,
            required_x100,
            ..
        } => Some((
            "F83".to_owned(),
            token.clone(),
            region.clone(),
            *measured_x100,
            *required_x100,
        )),
        _ => None,
    }
}

fn ratio_x100(a: [u8; 3], b: [u8; 3]) -> u32 {
    scaled(colour::contrast(a, b), 100.0)
}

/// A refusal the walk cannot settle: the validator's error and why the walk
/// stopped. A user import keeps it as a draft failure with no suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsettled {
    pub error: SkinError,
    pub why: String,
}

/// Where [`settle`] stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkResult {
    /// The package passes, with every change made.
    Passed(Canonical),
    /// A refusal the walk cannot settle; the changes before it still stand.
    Unsettled(Unsettled),
}

/// The token the skin sets for `id`: `id` itself when the skin sets it or
/// it derives from nothing, else (a derived token the skin omits, such as
/// `bg.dark` following `bg.primary`) the first source of its derivation the
/// skin sets, else `id` (which then takes its base default). `None` when
/// the skin may not set `id`.
fn owner(id: &str, tokens: &BTreeMap<String, TokenValue>) -> Option<String> {
    let e = entry_in(ENTRIES, id).filter(|_| settable(id))?;
    let source = e
        .derivation
        .filter(|_| !tokens.contains_key(id))
        .and_then(|d| {
            d.sources()
                .into_iter()
                .find(|s| tokens.contains_key(*s) && settable(s))
        });
    Some(source.unwrap_or(id).to_owned())
}

/// The colours a ground is made of, the opaque ground first, then its
/// tints from the outermost: `status.online 60.0% over bg.dark` →
/// `bg.dark`, `status.online`.
fn ground_parts(ground: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = ground
        .split(" over ")
        .map(|p| p.split(' ').next().unwrap_or(p))
        .collect();
    parts.rotate_right(1);
    parts
}

/// What the walk may move to fix `fg` on `ground`, in order, each with the
/// colour it moves away from: the text token itself, then the opaque ground
/// beneath a tint, then (when the text is a fixed role) the ground's
/// tints. A derived colour the skin omits is moved through the colour it
/// follows (plan E7.7.7 e777n: white on `status.online 60% over bg.dark`
/// moves the skin's `bg.primary`).
pub(crate) fn targets(
    fg: &str,
    ground: &str,
    tokens: &BTreeMap<String, TokenValue>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |target: Option<String>, other: &str| {
        if let Some(t) = target.filter(|t| !out.iter().any(|(o, _)| o == t)) {
            out.push((t, other.to_owned()));
        }
    };
    let parts = ground_parts(ground);
    if settable(fg) {
        push(Some(fg.to_owned()), ground);
        if parts.len() > 1 {
            push(owner(parts[0], tokens), fg);
        }
    } else {
        for part in parts {
            push(owner(part, tokens), fg);
        }
    }
    out
}

/// The first of [`targets`], or `None` when the pair has no colour the
/// skin may set.
pub(crate) fn target_of(
    fg: &str,
    ground: &str,
    tokens: &BTreeMap<String, TokenValue>,
) -> Option<(String, String)> {
    targets(fg, ground, tokens).into_iter().next()
}

/// Builds with `build`, validates, and walks every failing pair until the
/// package passes or a refusal cannot be settled.
///
/// # Errors
/// What `build` returns.
pub fn settle(
    tokens: &mut BTreeMap<String, TokenValue>,
    base: Base,
    build: impl Fn(&BTreeMap<String, TokenValue>) -> Result<Vec<u8>, ImportError>,
) -> Result<(WalkResult, Vec<Change>), ImportError> {
    let mut changes = Vec::new();
    loop {
        let err = match validate::run(&build(tokens)?) {
            Ok(c) => return Ok((WalkResult::Passed(c), changes)),
            Err(e) => e,
        };
        let stop = |why: &str| {
            WalkResult::Unsettled(Unsettled {
                error: err.clone(),
                why: why.to_owned(),
            })
        };
        let Some((rule, fg, ground, before, required)) = pair_of(&err.rule) else {
            return Ok((stop("not a contrast pair"), changes));
        };
        if changes.len() >= MAX_CHANGES {
            return Ok((stop("the walk made its limit of changes"), changes));
        }
        let tries = targets(&fg, &ground, tokens);
        if tries.is_empty() {
            return Ok((
                stop("neither colour of the pair is a token the skin sets"),
                changes,
            ));
        }
        let same = |e: &SkinError| {
            pair_of(&e.rule).is_some_and(|(r, f, g, _, _)| r == rule && f == fg && g == ground)
        };
        let mut found = None;
        for (target, other) in tries {
            let Some(original) = colour_of(tokens, base, &target) else {
                return Ok((stop("the token is not a resolved colour"), changes));
            };
            // Away from the other colour; text on an image region goes away
            // from the base's ground.
            let up = colour_of(tokens, base, &other)
                .map_or(matches!(base, Base::Dark | Base::HcDark), |o| {
                    colour::luminance(original) >= colour::luminance(o)
                });
            let mut trial = tokens.clone();
            let walked = lightness_walk(original, up, |c| {
                trial.insert(target.clone(), color(c));
                Ok(match validate::run(&build(&trial)?) {
                    Ok(_) => true,
                    Err(e) => !same(&e),
                })
            })?;
            if let Some(new) = walked {
                found = Some((target, original, new));
                break;
            }
        }
        let Some((target, original, new)) = found else {
            let why = format!("no lightness of {fg} or its ground passes {rule} on {ground}");
            return Ok((stop(&why), changes));
        };
        tokens.insert(target.clone(), color(new));
        let after_x100 = colour_of(tokens, base, &fg)
            .zip(colour_of(tokens, base, &ground))
            .map(|(f, g)| ratio_x100(f, g));
        changes.push(Change {
            token: target,
            rule,
            fg,
            ground,
            original,
            new,
            before_x100: before,
            after_x100,
            required_x100: required,
        });
    }
}

/// An opaque colour token value.
#[must_use]
pub fn color([r, g, b]: [u8; 3]) -> TokenValue {
    TokenValue::Color { r, g, b, a: 255 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips_and_walks_minimally() {
        for c in [[12, 51, 87], [255, 255, 255], [0, 0, 0], [26, 103, 175]] {
            assert_eq!(from_oklab(oklab(c)), c);
        }
        let ground = [22, 22, 22];
        let new = lightness_walk([12, 51, 87], true, |c| {
            Ok(colour::contrast(c, ground) >= 4.5)
        })
        .unwrap()
        .unwrap();
        assert!(colour::contrast(new, ground) >= 4.5);
        let [l, ..] = oklab(new);
        let less = from_oklab([l - 0.01, oklab(new)[1], oklab(new)[2]]);
        assert!(
            colour::contrast(less, ground) < 4.5,
            "{new:?} is not minimal"
        );
    }
}
