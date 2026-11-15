//! Validator step 3: a variant's token file, a restricted profile of W3C
//! DTCG 2025.10 (plan E7.7.1), parsed to numbers.
//!
//! * Types: `color` (`"#RRGGBB"` / `"#RRGGBBAA"`, or the 2025.10 object
//!   `{colorSpace: "srgb", components: [r, g, b], alpha?, hex?}`),
//!   `dimension` (`{value: <integer>, unit: "px"}`, in the token's range),
//!   `number` (≤ 3 decimals, in range) and `fontFamily` (one name of the
//!   bundled allow-list).
//! * A token is an object with `$value`; a group is any other object.
//!   Tokens and groups may carry `$type` (a group's is inherited),
//!   `$description` and `$extensions`, whose one namespace is
//!   [`EXTENSIONS_NAMESPACE`], an empty object in format 1. Every other
//!   `$` key (`$ref`, `$extends`, `$deprecated`, …) is refused, as is a
//!   name outside `[a-z0-9-]`.
//! * An alias (`"{group.token}"`) names a token of the same file; a chain
//!   that returns to itself is refused.
//! * Every token's path is an id of the closed registry, of the registry's
//!   type, and themeable.
//!
//! The first breach refuses the whole file: nothing is skipped and nothing
//! partly applied (unlike Telegram and Firefox, which skip bad keys).
//! CSS is never built from this text: [`emit`] re-emits from the numbers.

pub mod emit;
mod value;

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinRule, TokenValue};
use serde_json::{Map, Value};

use crate::registry::{entry_in, Class, Entry, ENTRIES};

/// The one `$extensions` namespace a token file may use.
pub const EXTENSIONS_NAMESPACE: &str = "app.rekindle";

/// Tokens in one file (the §6 limits ledger).
pub const MAX_TOKENS: u64 = 512;

/// Longest `$description`, in scalars.
const MAX_DESCRIPTION: usize = 280;

/// A token as read, before alias resolution.
struct Raw<'a> {
    ty: Option<&'a str>,
    value: &'a Value,
}

/// Parses a token file against the app's registry.
///
/// # Errors
/// The first rule the file breaks, with `entry` as the error's entry.
pub fn parse(entry: &str, bytes: &[u8]) -> Result<BTreeMap<String, TokenValue>, SkinError> {
    parse_with(ENTRIES, entry, bytes)
}

/// Parses a token file against `table`.
///
/// # Errors
/// The first rule the file breaks, with `entry` as the error's entry.
pub fn parse_with(
    table: &[Entry],
    entry: &str,
    bytes: &[u8],
) -> Result<BTreeMap<String, TokenValue>, SkinError> {
    let fail = |rule| SkinError::new(entry, rule);
    let root: Value = serde_json::from_slice(bytes).map_err(|e| {
        fail(SkinRule::TokenSyntax {
            detail: e.to_string(),
        })
    })?;
    let Value::Object(root) = &root else {
        return Err(fail(SkinRule::TokenSyntax {
            detail: "the file is not a JSON object".into(),
        }));
    };
    let mut raw = BTreeMap::new();
    walk(root, "", None, &mut raw).map_err(fail)?;
    let count = raw.len() as u64;
    if count > MAX_TOKENS {
        return Err(fail(SkinRule::TooManyTokens {
            count,
            max: MAX_TOKENS,
        }));
    }
    let mut out = BTreeMap::new();
    for path in raw.keys() {
        let e = entry_in(table, path).ok_or_else(|| {
            fail(SkinRule::UnknownToken {
                token: path.clone(),
            })
        })?;
        if e.class == Class::Fixed {
            return Err(fail(SkinRule::FixedToken {
                token: path.clone(),
            }));
        }
        let v = resolve(path, e, &raw).map_err(fail)?;
        out.insert(path.clone(), v);
    }
    Ok(out)
}

fn token_key(token: &str, key: &str) -> SkinRule {
    SkinRule::TokenKey {
        token: if token.is_empty() { "<root>" } else { token }.to_owned(),
        key: key.to_owned(),
    }
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The `$` keys shared by tokens and groups; `$value` is a token's own.
fn check_meta(path: &str, key: &str, v: &Value) -> Result<(), SkinRule> {
    match key {
        "$type" => v.as_str().map(|_| ()).ok_or_else(|| token_key(path, key)),
        "$description" => v
            .as_str()
            .filter(|s| s.chars().count() <= MAX_DESCRIPTION)
            .map(|_| ())
            .ok_or_else(|| SkinRule::BadTokenValue {
                token: path.to_owned(),
                expected: format!("a $description string of at most {MAX_DESCRIPTION} scalars"),
            }),
        "$extensions" => match v {
            Value::Object(ns) => match ns.iter().next() {
                None => Ok(()),
                Some((name, body)) if name == EXTENSIONS_NAMESPACE && ns.len() == 1 => body
                    .as_object()
                    .filter(|b| b.is_empty())
                    .map(|_| ())
                    .ok_or_else(|| token_key(path, &format!("$extensions.{name}"))),
                Some(_) => Err(token_key(
                    path,
                    &format!("$extensions (only {EXTENSIONS_NAMESPACE})"),
                )),
            },
            _ => Err(token_key(path, key)),
        },
        _ => Err(token_key(path, key)),
    }
}

fn walk<'a>(
    group: &'a Map<String, Value>,
    path: &str,
    inherited: Option<&'a str>,
    out: &mut BTreeMap<String, Raw<'a>>,
) -> Result<(), SkinRule> {
    let own_type = group.get("$type").and_then(Value::as_str);
    if let Some(value) = group.get("$value") {
        for (k, v) in group {
            if k != "$value" {
                check_meta(path, k, v)?;
            }
        }
        out.insert(
            path.to_owned(),
            Raw {
                ty: own_type.or(inherited),
                value,
            },
        );
        return Ok(());
    }
    let ty = own_type.or(inherited);
    for (k, v) in group {
        if k.starts_with('$') {
            check_meta(path, k, v)?;
            continue;
        }
        let child = if path.is_empty() {
            k.clone()
        } else {
            format!("{path}.{k}")
        };
        if !is_name(k) {
            return Err(token_key(&child, k));
        }
        let Value::Object(obj) = v else {
            return Err(SkinRule::BadTokenValue {
                token: child,
                expected: "a token or group object".into(),
            });
        };
        walk(obj, &child, ty, out)?;
    }
    Ok(())
}

/// An alias's target path, if `v` is `"{a.b}"`.
fn alias_target(v: &Value) -> Option<&str> {
    let s = v.as_str()?.strip_prefix('{')?.strip_suffix('}')?;
    s.split('.').all(is_name).then_some(s)
}

/// Follows `path`'s alias chain to a concrete value and parses it as `e`'s
/// type.
fn resolve(path: &str, e: &Entry, raw: &BTreeMap<String, Raw<'_>>) -> Result<TokenValue, SkinRule> {
    let mut seen = vec![path];
    let mut cur = &raw[path];
    let mut ty = cur.ty;
    while let Some(target) = alias_target(cur.value) {
        if seen.contains(&target) {
            return Err(SkinRule::AliasCycle {
                token: path.to_owned(),
            });
        }
        cur = raw.get(target).ok_or_else(|| SkinRule::UnresolvedAlias {
            token: path.to_owned(),
            target: target.to_owned(),
        })?;
        seen.push(target);
        if let (Some(a), Some(b)) = (ty, cur.ty) {
            if a != b {
                return Err(mismatch(path, b, a));
            }
        }
        ty = ty.or(cur.ty);
    }
    let expected = e.ty.dtcg();
    match ty {
        Some(t) if t == expected => value::parse(path, e.ty, cur.value),
        Some(t) => Err(mismatch(path, expected, t)),
        None => Err(SkinRule::BadTokenValue {
            token: path.to_owned(),
            expected: format!("a $type ({expected}) on the token or a group above it"),
        }),
    }
}

fn mismatch(path: &str, expected: &str, found: &str) -> SkinRule {
    SkinRule::TokenTypeMismatch {
        token: path.to_owned(),
        expected: expected.to_owned(),
        found: found.chars().take(32).collect(),
    }
}
