//! `check-duplicate-constants`: one `SCREAMING_CASE` name, one value.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use quote::ToTokens as _;

use crate::walk_source_files;

// ────────────────────────────────────────────────────────────────
// check-duplicate-constants
// ────────────────────────────────────────────────────────────────
//
// Fails when one `SCREAMING_CASE` name is declared in two or more
// crates with two or more different values. That is the shape of the
// drift this workspace keeps producing: two tracks (desktop
// `rekindle-protocol` / daemon `rekindle-transport`) implementing one
// protocol, each with its own copy of a shared parameter.
//
// It caught `DEFAULT_TTL` (codec 5 vs transport 3) — a gossip hop
// budget that made daemon-originated messages reach fewer members of
// the same mesh. Note the limit that finding also exposed: the sibling
// bug was `fanout_degree`, a *function*, and no constant gate can see
// it. Cross-implementation parity tests are the durable protection;
// this is the cheap net under them.

/// Names permitted to differ across crates, each with the reason.
///
/// Kept deliberately short. An entry here is a claim that two values
/// under one name are *supposed* to disagree — if that is not true, the
/// fix is to converge the values, not to add a row.
const DUPLICATE_CONSTANT_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "NONCE_LEN",
        "XChaCha20-Poly1305 takes a 24-byte nonce, AES-256-GCM a 12-byte one. \
         Two ciphers, two correct answers.",
    ),
    (
        "HKDF_INFO",
        "Per-purpose HKDF domain-separation labels MUST differ — identical \
         labels across contexts is the bug, not the divergence.",
    ),
    (
        "SCAN_PARALLELISM",
        "Unrelated scans with unrelated cost profiles: presence fan-out in \
         src-tauri vs join-stage segment probing in governance-runtime.",
    ),
];

/// One declaration site.
struct ConstSite {
    crate_name: String,
    value: String,
    rel_path: String,
    line: usize,
}

/// Which crate a source file belongs to, for grouping.
pub(crate) fn crate_of(rel_path: &str) -> String {
    let mut parts = rel_path.split('/');
    match (parts.next(), parts.next()) {
        (Some("crates"), Some(name)) => name.to_string(),
        (Some("src-tauri"), _) => "rekindle (src-tauri)".to_string(),
        _ => rel_path.to_string(),
    }
}

/// True when `value` is a re-export alias of `name` — `pub const X: T =
/// some::path::X;` (optionally cast). Those are definitionally equal to
/// their source and must not count as a competing value; without this
/// every deliberate alias, such as the permission re-exports in
/// `rekindle-voice/src/signaling/deps.rs`, would be a false failure.
fn is_reexport_alias(name: &str, value: &str) -> bool {
    // `to_token_stream` spaces punctuation out: `a :: b :: NAME as usize`.
    let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    let path = compact.split("as").next().unwrap_or(&compact);
    path.contains("::") && path.rsplit("::").next() == Some(name)
}

/// Collect `const` / `static` declarations, skipping `#[cfg(test)]`
/// items — a test fixture is allowed to pick its own numbers.
fn collect_consts(
    items: &[syn::Item],
    src: &str,
    rel_path: &str,
    out: &mut Vec<(String, ConstSite)>,
) {
    fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|a| {
            a.path().is_ident("cfg")
                && a.to_token_stream()
                    .to_string()
                    .replace(' ', "")
                    .contains("test")
        })
    }

    for item in items {
        match item {
            syn::Item::Mod(m) if !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    collect_consts(inner, src, rel_path, out);
                }
            }
            syn::Item::Const(c) if !is_cfg_test(&c.attrs) => {
                push_site(&c.ident, &c.expr, src, rel_path, out);
            }
            syn::Item::Static(s) if !is_cfg_test(&s.attrs) => {
                push_site(&s.ident, &s.expr, src, rel_path, out);
            }
            _ => {}
        }
    }
}

fn push_site(
    ident: &syn::Ident,
    expr: &syn::Expr,
    src: &str,
    rel_path: &str,
    out: &mut Vec<(String, ConstSite)>,
) {
    let name = ident.to_string();
    if name.len() < 2
        || !name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return;
    }
    // Display line only — syn decides what counts as a declaration, so a
    // miss here costs a nicer error message, never a wrong verdict.
    let line = src
        .lines()
        .position(|l| {
            let t = l.trim_start();
            (t.starts_with("const ")
                || t.starts_with("static ")
                || t.starts_with("pub const ")
                || t.starts_with("pub static ")
                || t.contains(") const ")
                || t.contains(") static "))
                && t.contains(&name)
        })
        .map_or(0, |i| i + 1);
    out.push((
        name,
        ConstSite {
            crate_name: crate_of(rel_path),
            value: expr.to_token_stream().to_string(),
            rel_path: rel_path.to_string(),
            line,
        },
    ));
}

pub(crate) fn check_duplicate_constants(root: &Path) -> Result<()> {
    let mut by_name: BTreeMap<String, Vec<ConstSite>> = BTreeMap::new();

    for subdir in ["crates", "src-tauri/src"] {
        for path in walk_source_files(&root.join(subdir), &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            // Fixtures and micro-benchmarks legitimately pick their own
            // constants; they are not shipped protocol.
            if rel.contains("/tests/") || rel.contains("/benches/") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            // A file that does not parse is a compiler problem, not this
            // gate's — cargo will say so far more usefully.
            let Ok(parsed) = syn::parse_file(&src) else {
                continue;
            };
            let mut found = Vec::new();
            collect_consts(&parsed.items, &src, &rel, &mut found);
            for (name, site) in found {
                by_name.entry(name).or_default().push(site);
            }
        }
    }

    let mut offenders = 0usize;
    for (name, sites) in &by_name {
        if DUPLICATE_CONSTANT_EXCEPTIONS.iter().any(|(n, _)| n == name) {
            continue;
        }
        let real: Vec<&ConstSite> = sites
            .iter()
            .filter(|s| !is_reexport_alias(name, &s.value))
            .collect();
        let crates: std::collections::BTreeSet<&str> =
            real.iter().map(|s| s.crate_name.as_str()).collect();
        let values: std::collections::BTreeSet<&str> =
            real.iter().map(|s| s.value.as_str()).collect();
        if crates.len() < 2 || values.len() < 2 {
            continue;
        }
        println!(
            "  ✗ {name} — {} crates, {} values",
            crates.len(),
            values.len()
        );
        for site in &real {
            println!(
                "      {:32} = {}\n          {}:{}",
                site.crate_name, site.value, site.rel_path, site.line
            );
        }
        offenders += 1;
    }

    if offenders > 0 {
        return Err(anyhow!(
            "{offenders} constant name(s) declared in 2+ crates with different values.\n\
             Converge them on one declaration and have the others import it.\n\
             If they are genuinely supposed to differ, add the name to\n\
             DUPLICATE_CONSTANT_EXCEPTIONS in xtask/src/main.rs with the reason."
        ));
    }
    Ok(())
}
