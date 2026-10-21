//! `check-duplicate-bodies`: no copy-pasted function bodies.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::duplicate_constants::crate_of;
use crate::walk_source_files;

// ────────────────────────────────────────────────────────────────
// check-duplicate-bodies
// ────────────────────────────────────────────────────────────────
//
// Fails when two crates contain a function with a byte-identical body
// (comments and whitespace normalised away). Names may differ — that is
// the point: `u32_to_role_id` and `role_id_to_key` were the same
// function under two names in two crates, and `veilid_update_name` /
// `veilid_update_label` still are.
//
// This is the general form of what `check-duplicate-constants` catches
// for values only. The manual dedup audit that produced most of this
// branch missed several of these; a gate does not get tired.
//
// Bodies shorter than MIN_BODY_CHARS are ignored — one-line delegations
// (`self.foo_impl(x)`) legitimately repeat across adapter impls and
// would drown the signal. At the current threshold the workspace yields
// eleven findings and zero false positives.

/// Normalised body length below which a duplicate is not interesting.
const MIN_BODY_CHARS: usize = 50;

/// Duplicate bodies allowed for now, keyed by the exact set of
/// `crate::function` sites, with the reason and the work that removes it.
///
/// **This list must only shrink.** An entry is a promise to converge,
/// not an exemption. Keying on the full site set rather than a bare
/// function name keeps it precise: exempting every `generate` in the
/// workspace would be a hole, exempting exactly these two is a note.
const DUPLICATE_BODY_EXCEPTIONS: &[(&[&str], &str)] = &[
    (
        &[
            "rekindle (src-tauri)::from",
            "rekindle (src-tauri)::from_dto",
        ],
        "`RoleDefinition::from_dto` and `RoleDto::from` build two \
         different types from the same source, field by field, so the \
         bodies match. The types must not be merged: `RoleDto` \
         serialises `permissions` as a string \
         (`serialize_u64_as_string`) because a u64 above 2^53-1 loses \
         low bits through JavaScript's Number, which silently strips \
         ADMINISTRATOR (bit 3) from the Owner role. `RoleDefinition` is \
         cached governance state and carries no such wire constraint. \
         Routing one through the other needs a third field-by-field \
         conversion, which is the same code again.",
    ),
    (
        &["rekindle (src-tauri)::as_ref", "rekindle-protocol::as_str"],
        "Two ChannelType enums, not two functions: the bodies match \
         because the types do. Converging is a Tier-1 type migration \
         (same shape as the onboarding DTOs in Phase 1), not a helper \
         hoist. Plan 4.9.",
    ),
];

/// Is this body a single delegating call and nothing else?
///
/// The adapter pattern makes these unavoidable and duplicate-looking:
/// six different `Deps` traits each declare `increment_lamport`, and
/// each impl is one line forwarding to the *same*
/// `state_helpers::increment_lamport`. The shared logic already has one
/// home; the repetition is the trait signatures, which cannot be
/// deduplicated and carry no logic that can drift.
///
/// Deliberately narrow: one expression, no `;`, no control flow, no
/// closure. `if x { a() } else { b() }` is not a delegation, and
/// anything with two statements has somewhere for the copies to
/// diverge — which is the whole point of the gate.
fn is_pure_delegation(body: &str) -> bool {
    // A `()`-returning delegation ends in `;` — strip exactly one, then
    // require none remain. Two statements have somewhere to diverge.
    let body = body.strip_suffix(';').unwrap_or(body);
    if body.contains(';') || body.contains('|') {
        return false;
    }
    for kw in ["if", "match", "for", "while", "loop", "unsafe"] {
        if body.starts_with(kw) && body[kw.len()..].starts_with(|c: char| !c.is_alphanumeric()) {
            return false;
        }
    }
    let Some(open) = body.find('(') else {
        return false;
    };
    // Everything before the first `(` must be a path: `a::b::c` / `self.x`.
    let callee = &body[..open];
    !callee.is_empty()
        && callee
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '&' | '<' | '>' | ' '))
        && (body.ends_with(')') || body.ends_with(").await") || body.ends_with(")?"))
}

pub(crate) fn normalise_body(body: &str) -> String {
    let no_comments: String = body
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    no_comments.split_whitespace().collect()
}

/// Extract `(name, normalised_body)` for every free/`impl` function in a
/// file, skipping anything after the first `#[cfg(test)]`.
fn functions_in(src: &str) -> Vec<(String, String, usize)> {
    let cut = src.find("#[cfg(test)]").unwrap_or(src.len());
    let src = &src[..cut];
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut search = 0usize;

    while let Some(rel) = src[search..].find("fn ") {
        let at = search + rel;
        search = at + 3;
        // Require a word boundary before `fn`.
        if at > 0 && !matches!(bytes[at - 1], b' ' | b'\n' | b'\t') {
            continue;
        }
        let rest = &src[at + 3..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let Some(open) = src[at..].find('{') else {
            continue;
        };
        let open = at + open;
        let mut depth = 0usize;
        let mut close = open;
        for (i, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        if close <= open {
            continue;
        }
        let body = normalise_body(&src[open + 1..close]);
        if body.len() >= MIN_BODY_CHARS && !is_pure_delegation(&body) {
            let line = src[..at].matches('\n').count() + 1;
            out.push((name, body, line));
        }
        search = close;
    }
    out
}

pub(crate) fn check_duplicate_bodies(root: &Path) -> Result<()> {
    crate::duplication::report(
        "function-body",
        collect_duplicate_bodies(root)?,
        root,
        "Hoist the shared one to a crate both can reach, or make the \
         existing definition `pub` and import it. A \"must stay in sync \
         with X\" comment is not a fix — that comment IS the defect.",
    )
}

pub(crate) fn collect_duplicate_bodies(root: &Path) -> Result<Vec<crate::duplication::Offender>> {
    let mut by_body: BTreeMap<String, Vec<(String, String, String, usize)>> = BTreeMap::new();

    for subdir in ["crates", "src-tauri/src"] {
        for path in walk_source_files(&root.join(subdir), &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if rel.contains("/tests/") || rel.contains("/benches/") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (name, body, line) in functions_in(&src) {
                by_body
                    .entry(body)
                    .or_default()
                    .push((crate_of(&rel), name, rel.clone(), line));
            }
        }
    }

    let mut offenders = Vec::new();
    for sites in by_body.values() {
        let crates: std::collections::BTreeSet<&str> =
            sites.iter().map(|(c, _, _, _)| c.as_str()).collect();
        // Two or more distinct FILES, not two or more crates. The gate
        // used to skip same-crate duplicates entirely, which made
        // `build_adapter` written out three times in src-tauri
        // invisible to it — the largest category of duplication left in
        // the tree after the cross-crate ones were burned down.
        let files: std::collections::BTreeSet<&str> =
            sites.iter().map(|(_, _, f, _)| f.as_str()).collect();
        if files.len() < 2 {
            continue;
        }
        // Key on the exact set of `crate::function` sites, not a bare
        // name: exempting every `generate` in the workspace would be a
        // hole; exempting exactly these two sites is a note.
        let mut key: Vec<String> = sites
            .iter()
            .map(|(c, n, _, _)| format!("{c}::{n}"))
            .collect();
        key.sort();
        key.dedup();
        if DUPLICATE_BODY_EXCEPTIONS.iter().any(|(allowed, _)| {
            allowed.len() == key.len()
                && allowed
                    .iter()
                    .zip(key.iter())
                    .all(|(a, k)| *a == k.as_str())
        }) {
            continue;
        }
        let mut lines = vec![if crates.len() > 1 {
            format!("  ✗ identical body in {} crates", crates.len())
        } else {
            format!("  ✗ identical body in {} files of one crate", files.len())
        }];
        for (crate_name, name, rel, line) in sites {
            lines.push(format!("      {crate_name:28} {name:32} {rel}:{line}"));
        }
        let mut sorted_files: Vec<&str> = files.iter().copied().collect();
        sorted_files.sort_unstable();
        offenders.push(crate::duplication::Offender {
            key: format!("body {}", sorted_files.join(" ")),
            lines,
        });
    }

    Ok(offenders)
}
