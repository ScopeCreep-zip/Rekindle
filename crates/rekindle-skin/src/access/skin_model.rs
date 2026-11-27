//! The full pair model ([`super::model`]) on a skin's resolved variant:
//! each pair measured on the variant's own tokens, named in registry ids.
//!
//! The variant is what [`crate::registry::resolve`] serves over the table
//! the validator runs on: over [`crate::registry::ENTRIES`] a package that
//! lacks the dialog roles is served them (`dialog::fill` and the one dialog
//! walk; ST1 rulings b, j), so the model's dialog pairs are measured on the
//! served roles. The import's base walk runs over
//! [`crate::registry::BASE_TABLE`], which holds the dialog roles out until
//! they are derived from the settled base (ST1 ruling 2); a pair reading a
//! held-out row is not a pair of that table, as the dialog rows' own
//! constraints are not.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::dto::skin::{SkinRule, TokenValue};

use super::model::{self, var_name, var_refs, ModelPair, Rgb};
use super::Measured;
use crate::registry::{TokenEntry, TokenType, ENTRIES};
use crate::tokens::emit::css_value;

/// The variant as the CSS reads it: custom property → value, custom
/// property → registry id, and the registry rows its table holds out.
struct Sheet<'a> {
    values: BTreeMap<String, String>,
    ids: BTreeMap<&'a str, &'a str>,
    held_out: BTreeSet<&'static str>,
}

impl<'a> Sheet<'a> {
    fn new(resolved: &'a [(&'a TokenEntry, TokenValue)]) -> Self {
        let mut values = BTreeMap::new();
        let mut ids = BTreeMap::new();
        for (e, v) in resolved {
            if !matches!(e.ty, TokenType::Color | TokenType::Fraction { .. }) {
                continue;
            }
            if let Some(value) = css_value(e, v) {
                values.insert(e.css.to_owned(), value);
                ids.insert(e.css, e.id);
            }
        }
        let held_out = ENTRIES
            .iter()
            .filter(|e| !resolved.iter().any(|(r, _)| r.id == e.id))
            .map(|e| e.css)
            .collect();
        Self {
            values,
            ids,
            held_out,
        }
    }

    /// Whether `pair` reads a registry row the variant's table holds out:
    /// a `var()` of its value or a token of its ground.
    fn holds_out(&self, pair: &ModelPair<'_>) -> bool {
        let value = var_refs(pair.value);
        let ground = pair
            .ground
            .split_whitespace()
            .map(|w| model::share_token(w).unwrap_or(w));
        value
            .iter()
            .map(String::as_str)
            .chain(ground)
            .any(|name| self.held_out.contains(name))
    }

    /// Why `pair` cannot be measured: its opaque ground drawn translucent,
    /// else the measure's own reason (`why`).
    fn unmeasured(&self, pair: &ModelPair<'_>, why: String) -> SkinRule {
        match model::parse_ground(pair.ground) {
            Ok(g) if model::translucent(&self.values, g.base) => SkinRule::TranslucentGround {
                token: self.id(g.base).to_owned(),
            },
            _ => SkinRule::UnmeasuredPair {
                rule: rule_of(pair),
                why,
            },
        }
    }

    fn id<'s>(&self, css: &'s str) -> &'s str
    where
        'a: 's,
    {
        self.ids.get(css).copied().unwrap_or(css)
    }

    /// A declared ground in registry ids: `accent.fill 30% over bg.panel`.
    fn describe(&self, ground: &str) -> String {
        ground
            .split_whitespace()
            .map(|w| match model::share_token(w) {
                Some(share) => self.id(share),
                None => self.id(w),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn byte(c: f64) -> u8 {
    let v = (c.clamp(0.0, 1.0) * 255.0).round();
    // In 0..=255 after the clamp.
    format!("{v:.0}").parse().unwrap_or(u8::MAX)
}

fn bytes(rgb: Rgb) -> [u8; 3] {
    rgb.map(byte)
}

/// The WCAG criterion and the selector a pair is named by:
/// `1.4.3 .reply-snippet`, `1.4.11 .status-dot-online icon hovered`.
fn rule_of(pair: &ModelPair<'_>) -> String {
    let wcag = if pair.graphic { "1.4.11" } else { "1.4.3" };
    let state = match pair.state {
        "text" => String::new(),
        s => format!(" {}", s.trim_start_matches("text ")),
    };
    format!("{wcag} {}{state}", pair.selector)
}

/// Every model pair of the variant's table: measured, or why it cannot be
/// (a translucent ground, or [`SkinRule::UnmeasuredPair`] with the
/// measure's reason).
pub(super) fn model_pairs(
    resolved: &[(&TokenEntry, TokenValue)],
) -> Vec<Result<Measured, SkinRule>> {
    pairs_on(model::model(), resolved)
}

/// [`model_pairs`] for `pairs`.
fn pairs_on(
    pairs: &[ModelPair<'_>],
    resolved: &[(&TokenEntry, TokenValue)],
) -> Vec<Result<Measured, SkinRule>> {
    let sheet = Sheet::new(resolved);
    pairs
        .iter()
        .filter(|pair| !sheet.holds_out(pair))
        .map(|pair| {
            let ground = sheet.describe(pair.ground);
            let m = pair
                .measure(&sheet.values)
                .map_err(|why| sheet.unmeasured(pair, why))?;
            let fg = var_name(&sheet.values, pair.value).map_or(pair.value, |css| sheet.id(css));
            Ok(Measured {
                rule: rule_of(pair),
                fg: fg.to_owned(),
                ground,
                fg_rgb: bytes(m.fg),
                ground_rgb: bytes(m.ground),
                ratio: m.ratio,
                required_x100: if pair.graphic { 300 } else { 450 },
            })
        })
        .collect()
}

/// The model's refusals on the variant.
pub(super) fn model_failures(resolved: &[(&TokenEntry, TokenValue)]) -> Vec<SkinRule> {
    model_pairs(resolved)
        .into_iter()
        .filter_map(|r| match r {
            Ok(m) => m.refusal(),
            Err(rule) => Some(rule),
        })
        .collect()
}

#[cfg(test)]
#[path = "skin_model_tests.rs"]
mod tests;
