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
//! which only white clears). The base walk (`import::walk`) never crosses.
//! Each walk has its own limit of [`MAX_CHANGES`].

use std::collections::BTreeMap;
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
pub struct Settled {
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

/// The smallest move that settles `m`: every side away from the other
/// colour first, then every side through it.
fn first_move(
    tokens: &BTreeMap<String, TokenValue>,
    base: Base,
    m: &Measured,
    movable: &dyn Fn(&str) -> bool,
) -> Option<(String, [u8; 3], [u8; 3])> {
    let sides = sides(m, movable);
    for cross in [false, true] {
        for (token, other) in &sides {
            let Some(original) = rgb_of(tokens, base, token) else {
                continue;
            };
            let away = colour::luminance(original) >= colour::luminance(*other);
            let mut trial = tokens.clone();
            let walked = lightness::walk(original, away != cross, |c| {
                trial.insert(token.clone(), lightness::color(c));
                let passes = pairs(&trial, base).iter().any(|p| same(p, m) && p.passes());
                Ok::<_, Infallible>(passes)
            });
            if let Ok(Some(new)) = walked {
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
) -> Settled {
    let mut done = Settled::default();
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
                "no lightness of {} or its ground passes {} on {}",
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
mod tests {
    use super::*;
    use crate::registry::dialog;

    /// Lead round 4 c: the crossing retry moves only a dialog role. Text
    /// darker than a mid-grey selection that black cannot clear crosses
    /// to white; the pack's `accent.fill` and every other base token stay.
    #[test]
    fn the_crossing_retry_never_moves_a_base_token() {
        let mut t = BTreeMap::from([
            (
                "accent.fill".to_owned(),
                lightness::color([0x6a, 0x6a, 0x6a]),
            ),
            (
                "selection.text".to_owned(),
                lightness::color([0x50, 0x50, 0x50]),
            ),
        ]);
        let before = t.clone();
        let done = settle(&mut t, Base::Dark, &dialog::is_role);
        assert_eq!(done.unsettled, None);
        let moved = done
            .changes
            .iter()
            .find(|c| c.token == "selection.text")
            .expect("selection.text crosses");
        assert!(colour::luminance(moved.new) > colour::luminance(moved.original));
        for c in &done.changes {
            assert!(
                dialog::is_role(&c.token),
                "{} is not a dialog role",
                c.token
            );
        }
        for (id, v) in &before {
            assert!(dialog::is_role(id) || t.get(id) == Some(v), "{id} moved");
        }
        assert!(t
            .keys()
            .all(|id| before.contains_key(id) || dialog::is_role(id)));
    }
}
