//! The dialog walk (ST1 rulings 1, 2; lead round 4 b, c; plan E7.7.3 "the
//! editor's minimal Oklab-lightness walk per failing pair, every change
//! recorded"): a failing contrast pair that names a dialog role the caller
//! lets move is settled by moving that role's lightness, never a pack
//! colour.
//!
//! One function, two callers. The importer runs it after it derives the
//! roles the pack does not name ([`super::dialog::fill`]), with every role
//! movable, and records each move in the NOTICE. [`super::resolve`] runs it
//! at load with only the roles the package lacks movable: derived dialog
//! roles are ours, not the pack's, so a package imported before the roles
//! existed (xfire-classic) is served roles that pass, while a role a
//! package sets is held to the validator as written.
//!
//! The validator's own pair measure ([`access::measured`]) is the oracle.
//! For a failing pair the walk tries each movable side (the text, then its
//! ground) moved away from the other colour, the smallest move after which
//! the pair passes ([`lightness::walk`]); only when no side passes that way
//! does it try through the other colour (text darker than a mid-tone fill,
//! which only white clears). When no lightness at the role's own chroma
//! passes either way (lead round 5 e: winamp's `#00ff00` text derives a
//! `selection.text` whose darkest lightness at that chroma clamps to
//! `#000b00`), the role loses chroma: its Oklab a and b shrink toward the
//! achromatic axis at its own hue, the direction CSS Color 4's gamut
//! mapping takes in OKLCh, keeping the most chroma at which some lightness
//! passes; pure black or white ends that path ([`reduced`]). A move never
//! breaks a pair that passed before it: a role drawn on two grounds (XGlow's
//! `selection.text` on `accent.fill` and on the hovered primary button's
//! darker ground) settles at the colour both accept, never alternating
//! between one pair's smallest move and the other's. The base walk
//! (`import::walk`) never crosses and never reduces chroma. Each walk has
//! its own limit of [`MAX_CHANGES`].

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use rekindle_types::dto::skin::TokenValue;

use super::{resolve_unwalked, Base, ENTRIES};
use crate::access::{self, scaled, Measured};
use crate::colour;
use crate::lightness;

/// The dialog walk's limit of changes (the base walk has its own).
pub const MAX_CHANGES: usize = 64;

/// One change a walk made.
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

/// What [`settle`] did: every move, and why it stopped short when a
/// failing pair it may move could not be settled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DialogSettled {
    pub changes: Vec<Change>,
    pub unsettled: Option<String>,
}

/// The colours a ground is made of, the opaque ground first, then its
/// tints from the outermost: `status.online 60.0% over bg.dark` →
/// `bg.dark`, `status.online`.
#[must_use]
pub fn ground_parts(ground: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = ground
        .split(" over ")
        .map(|p| p.split(' ').next().unwrap_or(p))
        .collect();
    parts.rotate_right(1);
    parts
}

fn pairs(tokens: &BTreeMap<String, TokenValue>, base: Base) -> Vec<Measured> {
    access::measured(base, &resolve_unwalked(ENTRIES, base, tokens))
}

fn same(a: &Measured, b: &Measured) -> bool {
    a.rule == b.rule && a.fg == b.fg && a.ground == b.ground
}

/// A pair's identity: its rule, text and ground.
fn key(m: &Measured) -> (&str, &str, &str) {
    (&m.rule, &m.fg, &m.ground)
}

fn rgb_of(tokens: &BTreeMap<String, TokenValue>, base: Base, id: &str) -> Option<[u8; 3]> {
    resolve_unwalked(ENTRIES, base, tokens)
        .into_iter()
        .find(|(e, _)| e.id == id)
        .and_then(|(_, v)| match v {
            TokenValue::Color { r, g, b, .. } => Some([r, g, b]),
            _ => None,
        })
}

/// The movable sides of `m`, each with the colour it moves away from: the
/// text, then the parts of its ground.
fn sides(m: &Measured, movable: &dyn Fn(&str) -> bool) -> Vec<(String, [u8; 3])> {
    let mut out: Vec<(String, [u8; 3])> = Vec::new();
    if movable(&m.fg) {
        out.push((m.fg.clone(), m.ground_rgb));
    }
    for part in ground_parts(&m.ground) {
        if part != m.fg && movable(part) && !out.iter().any(|(t, _)| t == part) {
            out.push((part.to_owned(), m.fg_rgb));
        }
    }
    out
}

/// The lightness move of `original` (towards white when `up`) at the most
/// of its Oklab chroma for which some lightness passes, its hue kept; `None`
/// when even the achromatic end, pure black or white, does not pass.
fn reduced(
    original: [u8; 3],
    up: bool,
    mut passes: impl FnMut([u8; 3]) -> bool,
) -> Option<[u8; 3]> {
    let [l, a, b] = lightness::oklab(original);
    let end = if up { 1.0 } else { 0.0 };
    let at_end = |keep: f64| lightness::from_oklab([end, a * keep, b * keep]);
    if !passes(at_end(0.0)) {
        return None;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..lightness::STEPS {
        let mid = f64::midpoint(lo, hi);
        if passes(at_end(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lightness::walk_lab([l, a * lo, b * lo], up, |c| Ok::<_, Infallible>(passes(c)))
        .ok()
        .flatten()
}

/// The smallest move that settles `m` and keeps every pair that passes
/// now passing: every side away from the other colour first, then every
/// side through it, at the side's own chroma; only then the same with
/// chroma reduced ([`reduced`]).
fn first_move(
    tokens: &BTreeMap<String, TokenValue>,
    base: Base,
    m: &Measured,
    movable: &dyn Fn(&str) -> bool,
) -> Option<(String, [u8; 3], [u8; 3])> {
    let sides = sides(m, movable);
    let now = pairs(tokens, base);
    let passing: BTreeSet<(&str, &str, &str)> =
        now.iter().filter(|p| p.passes()).map(key).collect();
    for (reduce, cross) in [(false, false), (false, true), (true, false), (true, true)] {
        for (token, other) in &sides {
            let Some(original) = rgb_of(tokens, base, token) else {
                continue;
            };
            let away = colour::luminance(original) >= colour::luminance(*other);
            let mut trial = tokens.clone();
            let mut passes = |c: [u8; 3]| {
                trial.insert(token.clone(), lightness::color(c));
                let after = pairs(&trial, base);
                after.iter().any(|p| same(p, m) && p.passes())
                    && after
                        .iter()
                        .all(|p| p.passes() || !passing.contains(&key(p)))
            };
            let walked = if reduce {
                reduced(original, away != cross, passes)
            } else {
                lightness::walk(original, away != cross, |c| Ok::<_, Infallible>(passes(c)))
                    .ok()
                    .flatten()
            };
            if let Some(new) = walked {
                return Some((token.clone(), original, new));
            }
        }
    }
    None
}

/// Moves the roles `movable` admits until no failing pair names one.
pub fn settle(
    tokens: &mut BTreeMap<String, TokenValue>,
    base: Base,
    movable: &dyn Fn(&str) -> bool,
) -> DialogSettled {
    let mut done = DialogSettled::default();
    loop {
        let failing = pairs(tokens, base)
            .into_iter()
            .find(|m| !m.passes() && !sides(m, movable).is_empty());
        let Some(m) = failing else {
            return done;
        };
        if done.changes.len() >= MAX_CHANGES {
            done.unsettled = Some("the dialog walk made its limit of changes".to_owned());
            return done;
        }
        let Some((token, original, new)) = first_move(tokens, base, &m, movable) else {
            done.unsettled = Some(format!(
                "no lightness or chroma of {} or its ground passes {} on {}",
                m.fg, m.rule, m.ground
            ));
            return done;
        };
        tokens.insert(token.clone(), lightness::color(new));
        let after_x100 = pairs(tokens, base)
            .into_iter()
            .find(|p| same(p, &m))
            .map(|p| scaled(p.ratio, 100.0));
        done.changes.push(Change {
            token,
            before_x100: scaled(m.ratio, 100.0),
            required_x100: u32::from(m.required_x100),
            rule: m.rule,
            fg: m.fg,
            ground: m.ground,
            original,
            new,
            after_x100,
        });
    }
}

#[cfg(test)]
#[path = "dialog_walk_tests.rs"]
mod tests;
