//! The numeric re-emitter (plan E7.7.1 pipeline step 3): CSS values built
//! from parsed numbers alone, never from skin text. A colour, dimension or
//! number emits only characters of [`ALPHABET`]; a font emits the
//! app-owned stack of its allow-list entry. The custom-property grammar
//! admits `url()`, and a `}` escapes string-built CSS, so no byte a skin
//! wrote reaches a stylesheet.

use std::fmt::Write as _;

use rekindle_types::dto::skin::TokenValue;

use super::value::emit_thousandths;
use crate::registry::{font, TokenEntry, TokenType};

/// The characters a colour, dimension or number value may emit.
pub const ALPHABET: &str = "#0123456789abcdef.px-%";

/// Thousandths as a decimal number (`600` → `0.6`), for the JSON Schema.
#[must_use]
pub fn thousandths_number(t: i64) -> String {
    emit_thousandths(t)
}

/// Whether `s` is drawn from [`ALPHABET`].
#[must_use]
pub fn in_alphabet(s: &str) -> bool {
    s.chars().all(|c| ALPHABET.contains(c))
}

/// The CSS value of `v`, or `None` when it does not fit `e`'s type (a value
/// [`super::parse`] or the registry produced always fits).
#[must_use]
pub fn css_value(e: &TokenEntry, v: &TokenValue) -> Option<String> {
    match (e.ty, v) {
        (TokenType::Color, TokenValue::Color { r, g, b, a }) => Some(if *a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }),
        (TokenType::Dimension { min, max }, TokenValue::Dimension(px))
            if (min..=max).contains(px) =>
        {
            Some(format!("{px}px"))
        }
        (TokenType::Number { min, max }, TokenValue::Number(t)) if (min..=max).contains(t) => {
            Some(emit_thousandths(*t))
        }
        (TokenType::Fraction { min, max }, TokenValue::Number(t)) if (min..=max).contains(t) => {
            Some(format!("{}%", emit_thousandths(*t * 100)))
        }
        (TokenType::FontFamily, TokenValue::FontFamily(name)) => {
            font(name).map(|f| f.stack.to_owned())
        }
        _ => None,
    }
}

/// `  --prop: value;` lines for each resolved token, `indent` spaces deep.
/// `None` when a pair does not fit, which [`crate::registry::resolve`]'s
/// output never does; nothing is emitted partly.
#[must_use]
pub fn declarations(resolved: &[(&TokenEntry, TokenValue)], indent: usize) -> Option<String> {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for (e, v) in resolved {
        let value = css_value(e, v)?;
        let _ = writeln!(out, "{pad}{}: {value};", e.css);
    }
    Some(out)
}
