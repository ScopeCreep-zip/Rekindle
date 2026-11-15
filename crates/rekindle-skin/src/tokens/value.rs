//! One concrete `$value`, parsed to numbers by the registry type.

use rekindle_types::dto::skin::{SkinRule, TokenValue};
use serde_json::Value;

use crate::registry::{font, TokenType};

fn bad(token: &str, expected: impl Into<String>) -> SkinRule {
    SkinRule::BadTokenValue {
        token: token.to_owned(),
        expected: expected.into(),
    }
}

pub(super) fn parse(token: &str, ty: TokenType, v: &Value) -> Result<TokenValue, SkinRule> {
    match ty {
        TokenType::Color => color(token, v),
        TokenType::Dimension { min, max } => dimension(token, v, min, max),
        TokenType::Number { min, max } => number(token, v, min, max),
        TokenType::FontFamily => v
            .as_str()
            .and_then(font)
            .map(|f| TokenValue::FontFamily(f.name.to_owned()))
            .ok_or_else(|| bad(token, "a font name of the bundled allow-list")),
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `#RRGGBB` or `#RRGGBBAA`.
fn hex(s: &str) -> Option<[u8; 4]> {
    let digits = s.strip_prefix('#')?.as_bytes();
    if digits.len() != 6 && digits.len() != 8 {
        return None;
    }
    let mut out = [255u8; 4];
    for (o, pair) in out.iter_mut().zip(digits.chunks(2)) {
        *o = hex_digit(pair[0])? * 16 + hex_digit(pair[1])?;
    }
    Some(out)
}

/// A unit-interval number as a byte.
fn unit_byte(v: &Value) -> Option<u8> {
    let scaled = (v.as_f64().filter(|x| (0.0..=1.0).contains(x))? * 255.0).round();
    (0u8..=255).find(|byte| (f64::from(*byte) - scaled).abs() < 0.5)
}

fn color(token: &str, v: &Value) -> Result<TokenValue, SkinRule> {
    let expected =
        "#RRGGBB, #RRGGBBAA or {colorSpace: \"srgb\", components: [r, g, b], alpha?, hex?}";
    let rgba = match v {
        Value::String(s) => hex(s),
        Value::Object(o) => {
            let known = ["colorSpace", "components", "alpha", "hex"];
            let comps = o.get("components").and_then(Value::as_array);
            let rgb: Option<Vec<u8>> = comps
                .filter(|comp| comp.len() == 3)
                .and_then(|comp| comp.iter().map(unit_byte).collect());
            let alpha = match o.get("alpha") {
                None => Some(255),
                Some(a) => unit_byte(a),
            };
            let shape_ok = o.keys().all(|k| known.contains(&k.as_str()))
                && o.get("colorSpace").and_then(Value::as_str) == Some("srgb");
            match (shape_ok, rgb, alpha) {
                (true, Some(rgb_bytes), Some(alpha_byte)) => {
                    let rgba = [rgb_bytes[0], rgb_bytes[1], rgb_bytes[2], alpha_byte];
                    let hex_ok = match o.get("hex") {
                        None => true,
                        Some(h) => h
                            .as_str()
                            .and_then(hex)
                            .is_some_and(|parsed| parsed[..3] == rgba[..3] && parsed[3] == 255),
                    };
                    hex_ok.then_some(rgba)
                }
                _ => None,
            }
        }
        _ => None,
    };
    let [r, g, b, a] = rgba.ok_or_else(|| bad(token, expected))?;
    Ok(TokenValue::Color { r, g, b, a })
}

fn dimension(token: &str, v: &Value, min: i32, max: i32) -> Result<TokenValue, SkinRule> {
    let expected = format!("{{value: <integer {min}..={max}>, unit: \"px\"}}");
    let o = v.as_object().ok_or_else(|| bad(token, &expected))?;
    let px = o
        .get("value")
        .and_then(Value::as_i64)
        .filter(|_| o.len() == 2 && o.get("unit").and_then(Value::as_str) == Some("px"))
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| bad(token, &expected))?;
    Ok(TokenValue::Dimension(px))
}

fn number(token: &str, v: &Value, min: i64, max: i64) -> Result<TokenValue, SkinRule> {
    let expected = format!(
        "a number of at most 3 decimals in {}..={}",
        emit_thousandths(min),
        emit_thousandths(max)
    );
    let Value::Number(n) = v else {
        return Err(bad(token, &expected));
    };
    thousandths(&n.to_string())
        .filter(|t| (min..=max).contains(t))
        .map(TokenValue::Number)
        .ok_or_else(|| bad(token, &expected))
}

/// The JSON number text as exact thousandths: `-?digits(.digits{1,3})?`,
/// no exponent; read from the text, never through a float.
fn thousandths(text: &str) -> Option<i64> {
    let (neg, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    let frac = frac.trim_end_matches('0');
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || int.len() > 12 || frac.len() > 3 || !digits(int) || !digits(frac) {
        return None;
    }
    let scaled: i64 = format!("{int}{frac:0<3}").parse().ok()?;
    Some(if neg { -scaled } else { scaled })
}

/// Thousandths as a decimal, for messages and CSS (`-1.25`, `0.5`, `3`).
pub(crate) fn emit_thousandths(t: i64) -> String {
    let sign = if t < 0 { "-" } else { "" };
    let a = t.unsigned_abs();
    let (int, frac) = (a / 1000, a % 1000);
    if frac == 0 {
        format!("{sign}{int}")
    } else {
        let f = format!("{frac:03}");
        format!("{sign}{int}.{}", f.trim_end_matches('0'))
    }
}
