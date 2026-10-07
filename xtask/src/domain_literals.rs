//! `check-domain-literals`: every domain-separation label lives in
//! `crates/rekindle-types/src/domains.rs`.
//!
//! A KDF `info`, signature prefix or hash context written as a literal at
//! its call site cannot be checked for uniqueness — two purposes sharing a
//! label is exactly the cross-protocol hole domain separation exists to
//! close. The registry's tests reject duplicates and prefix collisions;
//! this gate makes sure nothing bypasses the registry.
//!
//! Every string and byte-string literal is inspected, including literals
//! inside macro invocations (`format!`, `assert_eq!`, …), via the token
//! stream. Comments are not literals and are ignored, so docs can still
//! quote a label.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::{literals, walk_source_files};

const REGISTRY: &str = "crates/rekindle-types/src/domains.rs";

/// Does `s` have the shape of a Rekindle domain label?
///
/// Matches the registry's naming rule (`rekindle-<purpose>-v<N>`), a label
/// prefix completed at runtime (`rekindle-…-`), HPKE-style
/// `rekindle-…/<N>`, BLAKE3-context style `rekindle v<N> …`, and the
/// `ReKindle<Word>` ratchet labels. Crate names (`rekindle-transport`),
/// URLs (`rekindle://`), tracing targets (`rekindle_video::send`) and
/// prose do not match.
pub(crate) fn is_label_shaped(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix("ReKindle") {
        return rest.starts_with(|c: char| c.is_ascii_uppercase());
    }
    if let Some(rest) = s.strip_prefix("rekindle v") {
        return rest.starts_with(|c: char| c.is_ascii_digit());
    }
    let Some(rest) = s.strip_prefix("rekindle-") else {
        return false;
    };
    if rest.is_empty() || rest.contains(char::is_whitespace) || rest.contains('{') {
        return false;
    }
    let versioned = |sep: &str| {
        rest.rsplit_once(sep)
            .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    rest.ends_with('-') || versioned("-v") || versioned("/")
}

pub(crate) fn check_domain_literals(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for subdir in ["crates", "src-tauri/src", "src-tauri/tests", "xtask/src"] {
        for path in walk_source_files(&root.join(subdir), &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            // The registry itself, and this gate's own shape tests.
            if rel == REGISTRY || rel.ends_with("xtask/src/domain_literals.rs") {
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
            literals::for_each(&parsed, |value, token| {
                if is_label_shaped(value) {
                    let line = literals::line_of(&src, token);
                    hits.push(format!("{rel}:{line}  {token}"));
                }
            });
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} domain-label literal(s) outside {REGISTRY}.\n\
         Add the label to the registry (rekindle-<purpose>-v<N>) and use the constant.",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::is_label_shaped;

    #[test]
    fn label_shapes() {
        for s in [
            "rekindle-gov-op-v1",
            "rekindle-voice-receiver-report-v1",
            "rekindle-slot-",
            "rekindle-mek/1",
            "rekindle v1 vault-sqlcipher",
            "ReKindleRootKey",
        ] {
            assert!(is_label_shaped(s), "{s}");
        }
        for s in [
            "rekindle",
            "Rekindle",
            "rekindle-transport",
            "rekindle-protocol",
            "rekindle-e2e-{}",
            "rekindle_video::send",
            "rekindle://invite/{}/{}/{}",
            "rekindle/veilid",
            "rekindle.vault",
            "rekindle=info,warn",
            "rekindle node started and attached",
            "personal-sync-record",
        ] {
            assert!(!is_label_shaped(s), "{s}");
        }
    }
}
