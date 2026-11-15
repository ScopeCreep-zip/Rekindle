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

use rekindle_types::dto::skin::{SkinRule, TokenValue};

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

/// Builds with `build`, validates, and walks every failing pair until the
/// package passes.
///
/// # Errors
/// A refusal that is not a contrast pair, a pair nothing settable can fix,
/// or what `build` returns.
pub fn settle(
    tokens: &mut BTreeMap<String, TokenValue>,
    base: Base,
    build: impl Fn(&BTreeMap<String, TokenValue>) -> Result<Vec<u8>, ImportError>,
) -> Result<(Canonical, Vec<Change>), ImportError> {
    let mut changes = Vec::new();
    loop {
        let err = match validate::run(&build(tokens)?) {
            Ok(c) => return Ok((c, changes)),
            Err(e) => e,
        };
        let refused = || ImportError::Refused(err.clone());
        let (rule, fg, ground, before, required) = pair_of(&err.rule).ok_or_else(refused)?;
        if changes.len() >= MAX_CHANGES {
            return Err(refused());
        }
        let (target, other) = if settable(&fg) {
            (fg.clone(), ground.clone())
        } else if settable(&ground) {
            (ground.clone(), fg.clone())
        } else {
            return Err(refused());
        };
        let original = colour_of(tokens, base, &target).ok_or_else(refused)?;
        // Away from the other colour; text on an image region goes away
        // from the base's ground.
        let up = colour_of(tokens, base, &other)
            .map_or(matches!(base, Base::Dark | Base::HcDark), |o| {
                colour::luminance(original) >= colour::luminance(o)
            });
        let same = |e: &rekindle_types::dto::skin::SkinError| {
            pair_of(&e.rule).is_some_and(|(r, f, g, _, _)| r == rule && f == fg && g == ground)
        };
        let mut trial = tokens.clone();
        let new = lightness_walk(original, up, |c| {
            trial.insert(target.clone(), color(c));
            Ok(match validate::run(&build(&trial)?) {
                Ok(_) => true,
                Err(e) => !same(&e),
            })
        })?
        .ok_or_else(|| ImportError::WalkExhausted {
            token: target.clone(),
            ground: other.clone(),
            rule: rule.clone(),
        })?;
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
