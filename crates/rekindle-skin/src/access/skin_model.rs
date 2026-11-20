//! The full pair model ([`super::model`]) on a skin's resolved variant:
//! each pair measured on the variant's own tokens, named in registry ids.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinRule, TokenValue};

use super::model::{self, var_name, ModelPair, Rgb};
use super::Measured;
use crate::registry::{TokenEntry, TokenType};
use crate::tokens::emit::css_value;

/// The variant as the CSS reads it: custom property → value, and
/// custom property → registry id.
struct Sheet<'a> {
    values: BTreeMap<String, String>,
    ids: BTreeMap<&'a str, &'a str>,
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
        Self { values, ids }
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

/// Every model pair on the variant: measured, or the translucent ground
/// that keeps it from being measured.
pub(super) fn model_pairs(
    resolved: &[(&TokenEntry, TokenValue)],
) -> Vec<Result<Measured, SkinRule>> {
    let sheet = Sheet::new(resolved);
    model::model()
        .iter()
        .map(|pair| {
            let ground = sheet.describe(pair.ground);
            let m = pair
                .measure(&sheet.values)
                .map_err(|_| SkinRule::TranslucentGround {
                    token: ground.clone(),
                })?;
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
mod tests {
    use std::collections::BTreeMap;

    use rekindle_types::dto::skin::{SkinRule, TokenValue};

    use super::{model_failures, model_pairs};
    use crate::access::failures;
    use crate::registry::{resolve, Base, ENTRIES};

    /// Every pair of the model resolves on every base's registry defaults:
    /// it names only registry tokens and opaque grounds.
    #[test]
    fn every_model_pair_resolves_on_the_registry_defaults() {
        for base in Base::ALL {
            let resolved = resolve(ENTRIES, base, &BTreeMap::new());
            let pairs = model_pairs(&resolved);
            assert!(pairs.len() > 100);
            let bad: Vec<_> = pairs.iter().filter_map(|p| p.as_ref().err()).collect();
            assert!(bad.is_empty(), "{}: {bad:?}", base.key());
        }
    }

    fn is_model(rule: &SkinRule) -> bool {
        matches!(rule, SkinRule::Contrast { rule, .. } if rule.contains(" ."))
    }

    /// A skin whose window ground is #161616 keeps the default accent
    /// text, which passes on every opaque ground but reads 4.44:1 on the
    /// accent tint `.search-panel-chip-active` is drawn on: the registry
    /// constraints and `PAIRS` pass it, the full model refuses it.
    #[test]
    fn the_validator_refuses_a_skin_whose_tint_pair_fails() {
        let base = Base::Dark;
        let ground = TokenValue::Color {
            r: 22,
            g: 22,
            b: 22,
            a: 255,
        };
        let tokens: BTreeMap<String, TokenValue> =
            [("bg.primary".to_owned(), ground)].into_iter().collect();
        let resolved = resolve(ENTRIES, base, &tokens);
        let all = failures(base, &resolved);
        assert!(!all.is_empty(), "the tint pair passes");
        assert!(all.iter().all(is_model), "{all:?}");
        assert_eq!(all, model_failures(&resolved));
        assert!(
            all.iter().any(|r| matches!(
                r,
                SkinRule::Contrast { token, ground, .. }
                    if token == "accent.text" && ground.contains(" over ")
            )),
            "{all:?}"
        );
    }
}
