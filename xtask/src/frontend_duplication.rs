//! `check-frontend-duplication`: repeated TS/TSX function bodies and
//! CSS declaration blocks under `src/` (see `crate::duplication`).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::duplicate_bodies::normalise_body;
use crate::duplication::{report, Offender};
use crate::walk_source_files;

/// A TS body must be at least this long to count. Short arrow functions
/// (`() => setOpen(false)`) collide constantly and mean nothing.
const MIN_TS_BODY_CHARS: usize = 120;

/// A CSS rule needs at least this many declarations before a repeat is
/// worth reporting. One shared `background:` across ten selectors is
/// theming; three-plus identical declarations is a missing utility.
const MIN_CSS_DECLS: usize = 3;

/// Frontend duplicates that are accepted rather than fixed.
///
/// Keyed on the sorted `file:name` sites.
const DUPLICATE_FRONTEND_EXCEPTIONS: &[(&[&str], &str)] = &[];

// ────────────────────────────────────────────────────────────────
// check-frontend-duplication
// ────────────────────────────────────────────────────────────────

/// Extract `(name, normalised_body, line)` for TS/TSX functions —
/// `function foo(…) {…}`, `const foo = (…) => {…}`, and object methods.
fn functions_in_ts(src: &str) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();

    for (idx, line) in src.lines().enumerate() {
        let t = line.trim_start();
        let name = parse_ts_fn_name(t);
        let Some(name) = name else { continue };

        // Take the brace-balanced body starting at this line.
        let start = src.lines().take(idx).map(|l| l.len() + 1).sum::<usize>();
        let Some(open_rel) = src[start..].find('{') else {
            continue;
        };
        let open = start + open_rel;
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
        if body.len() >= MIN_TS_BODY_CHARS {
            out.push((name, body, idx + 1));
        }
    }
    out
}

/// `function foo(` / `const foo = (` / `export async function foo(`.
fn parse_ts_fn_name(t: &str) -> Option<String> {
    let after_fn = t
        .strip_prefix("export async function ")
        .or_else(|| t.strip_prefix("export function "))
        .or_else(|| t.strip_prefix("async function "))
        .or_else(|| t.strip_prefix("function "));
    if let Some(rest) = after_fn {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return (!name.is_empty()).then_some(name);
    }
    // `const foo = (…) => {` / `const foo = async (…) => {`
    let rest = t
        .strip_prefix("export const ")
        .or_else(|| t.strip_prefix("const "))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    let after = rest.get(name.len()..)?;
    (after.contains("=>") || after.contains("= function")).then_some(name)
}

/// Extract `(normalised_declarations, line)` for every innermost CSS
/// rule — a `{…}` containing no nested braces, so `@media` wrappers
/// contribute their inner rules rather than one giant blob.
fn css_blocks_in(src: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let open = i;
        let mut j = open + 1;
        let mut nested = false;
        while j < bytes.len() && bytes[j] != b'}' {
            if bytes[j] == b'{' {
                nested = true;
                break;
            }
            j += 1;
        }
        if nested || j >= bytes.len() {
            i = open + 1;
            continue;
        }
        let inner = &src[open + 1..j];
        let mut decls: Vec<String> = inner
            .split(';')
            .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|d| !d.is_empty() && d.contains(':') && !d.starts_with("/*"))
            .collect();
        if decls.len() >= MIN_CSS_DECLS {
            decls.sort();
            let line = src[..open].matches('\n').count() + 1;
            out.push((decls.join(" ; "), line));
        }
        i = j + 1;
    }
    out
}

/// Fail on duplicated TS function bodies and duplicated CSS rules.
///
/// The frontend had no duplication gate of any kind; `src/` is 31k lines
/// of TypeScript and 8k of CSS, all of it previously unguarded.
pub fn check_frontend_duplication(root: &Path) -> Result<()> {
    report(
        "frontend",
        collect_frontend_duplication(root)?,
        root,
        "TypeScript: hoist the shared body to src/utils/ (a leaf every tier \
         may import) and call it from both sites.\n\
         CSS: give the rule one home — group the selectors into a single \
         rule, or promote it to a utility class. Repeating three \
         declarations in four files is how a hover state ends up fixed in \
         three of them.",
    )
}

pub fn collect_frontend_duplication(root: &Path) -> Result<Vec<Offender>> {
    let src_dir = root.join("src");
    let mut offenders: Vec<Offender> = Vec::new();

    // ── TypeScript ────────────────────────────────────────────────
    let mut ts_by_body: BTreeMap<String, Vec<(String, String, usize)>> = BTreeMap::new();
    for path in walk_source_files(&src_dir, &["ts", "tsx"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (name, body, line) in functions_in_ts(&text) {
            ts_by_body
                .entry(body)
                .or_default()
                .push((rel.clone(), name, line));
        }
    }
    for sites in ts_by_body.values() {
        let files: std::collections::BTreeSet<&str> =
            sites.iter().map(|(f, _, _)| f.as_str()).collect();
        if files.len() < 2 {
            continue;
        }
        let mut key: Vec<String> = sites.iter().map(|(f, n, _)| format!("{f}:{n}")).collect();
        key.sort();
        key.dedup();
        if DUPLICATE_FRONTEND_EXCEPTIONS.iter().any(|(allowed, _)| {
            allowed.len() == key.len() && allowed.iter().zip(key.iter()).all(|(a, k)| *a == k)
        }) {
            continue;
        }
        let mut lines = vec![format!("  ✗ identical TS body in {} files", files.len())];
        for (rel, name, line) in sites {
            lines.push(format!("      {name:32} {rel}:{line}"));
        }
        offenders.push(Offender {
            key: format!("ts {}", key.join(" ")),
            lines,
        });
    }

    // ── CSS ───────────────────────────────────────────────────────
    let mut css_by_block: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for path in walk_source_files(&src_dir, &["css"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (block, line) in css_blocks_in(&text) {
            css_by_block
                .entry(block)
                .or_default()
                .push((rel.clone(), line));
        }
    }
    for (block, sites) in &css_by_block {
        if sites.len() < 2 {
            continue;
        }
        let mut lines = vec![
            format!("  ✗ identical CSS rule at {} sites", sites.len()),
            format!("      {{ {block} }}"),
        ];
        for (rel, line) in sites {
            lines.push(format!("      {rel}:{line}"));
        }
        // Keyed on the rule text, not the sites: moving one of four
        // copies into a new file must not silently re-accept it.
        offenders.push(Offender {
            key: format!("css {block}"),
            lines,
        });
    }

    Ok(offenders)
}
