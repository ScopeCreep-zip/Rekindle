//! The minimal Oklab-lightness walk (plan E7.7.3: "a built-in that fails
//! the validator is adjusted by the editor's minimal Oklab-lightness walk
//! per failing pair, every change recorded"; there is no exception for our
//! own skin).
//!
//! The validator is the oracle: the package is rebuilt and run through
//! [`validate::run`]; on a contrast refusal the colour the skin may set
//! (the text token, or its ground when the text is a fixed role) keeps
//! its Oklab a and b and moves its lightness away from the other colour,
//! the smallest move (bisected to 2⁻²⁰ of the way, [`crate::lightness`])
//! after which the validator no longer names that pair. Repeated until the
//! package passes. This base walk never crosses through the other colour;
//! only the dialog walk ([`dialog_walk`]) does, and it moves only dialog
//! roles (ST1 ruling 1, lead round 4 c).

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinRule, TokenValue};

use super::ImportError;
use crate::access::scaled;
use crate::colour;
pub use crate::lightness::{color, walk as lightness_walk};
use crate::output::Canonical;
pub use crate::registry::dialog_walk::Change;
use crate::registry::dialog_walk::{self, ground_parts};
use crate::registry::{
    dialog, entry_in, resolve_unwalked, Base, Class, TokenType, BASE_TABLE, ENTRIES,
};
use crate::validate;

/// The base walk's limit of changes (the dialog walk has its own).
const MAX_CHANGES: usize = 64;

fn rgb_of(v: &TokenValue) -> Option<[u8; 3]> {
    match v {
        TokenValue::Color { r, g, b, .. } => Some([*r, *g, *b]),
        _ => None,
    }
}

fn colour_of(tokens: &BTreeMap<String, TokenValue>, base: Base, id: &str) -> Option<[u8; 3]> {
    resolve_unwalked(ENTRIES, base, tokens)
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
/// package passes or a refusal cannot be settled: the base walk with every
/// dialog role held out, then the roles the pack does not name derived from
/// the settled base, then the dialog walk (ST1 rulings 1, 2), which moves
/// only dialog roles, and the validator over every row.
///
/// # Errors
/// What `build` returns.
pub fn settle(
    tokens: &mut BTreeMap<String, TokenValue>,
    base: Base,
    build: impl Fn(&BTreeMap<String, TokenValue>) -> Result<Vec<u8>, ImportError>,
) -> Result<(WalkResult, Vec<Change>), ImportError> {
    let mut changes = Vec::new();
    let held: BTreeMap<String, TokenValue> = tokens
        .iter()
        .filter(|(id, _)| dialog::is_role(id))
        .map(|(id, v)| (id.clone(), v.clone()))
        .collect();
    tokens.retain(|id, _| !dialog::is_role(id));
    let based = walk(tokens, base, &build, &mut changes)?;
    tokens.extend(held);
    if let WalkResult::Unsettled(_) = based {
        return Ok((based, changes));
    }
    dialog::fill(tokens, base);
    let dialogs = dialog_walk::settle(tokens, base, &dialog::is_role);
    changes.extend(dialogs.changes);
    let result = match validate::run(&build(tokens)?) {
        Ok(c) => WalkResult::Passed(c),
        Err(error) => WalkResult::Unsettled(Unsettled {
            error,
            why: dialogs.unsettled.unwrap_or_else(|| {
                "the pair names no dialog role, and the dialog walk moves no pack colour".to_owned()
            }),
        }),
    };
    Ok((result, changes))
}

/// The base walk over the base rows ([`BASE_TABLE`]) until the package
/// passes them or a refusal cannot be settled.
fn walk(
    tokens: &mut BTreeMap<String, TokenValue>,
    base: Base,
    build: &impl Fn(&BTreeMap<String, TokenValue>) -> Result<Vec<u8>, ImportError>,
    changes: &mut Vec<Change>,
) -> Result<WalkResult, ImportError> {
    let check = |bytes: &[u8]| validate::run_on(bytes, BASE_TABLE);
    loop {
        let err = match check(&build(tokens)?) {
            Ok(c) => return Ok(WalkResult::Passed(c)),
            Err(e) => e,
        };
        let stop = |why: &str| {
            WalkResult::Unsettled(Unsettled {
                error: err.clone(),
                why: why.to_owned(),
            })
        };
        let Some((rule, fg, ground, before, required)) = pair_of(&err.rule) else {
            return Ok(stop("not a contrast pair"));
        };
        if changes.len() >= MAX_CHANGES {
            return Ok(stop("the walk made its limit of changes"));
        }
        let tries = targets(&fg, &ground, tokens);
        if tries.is_empty() {
            return Ok(stop("neither colour of the pair is a token the skin sets"));
        }
        let same = |e: &SkinError| {
            pair_of(&e.rule).is_some_and(|(r, f, g, _, _)| r == rule && f == fg && g == ground)
        };
        let mut found = None;
        for (target, other) in tries {
            let Some(original) = colour_of(tokens, base, &target) else {
                return Ok(stop("the token is not a resolved colour"));
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
                Ok(match check(&build(&trial)?) {
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
            return Ok(stop(&why));
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
