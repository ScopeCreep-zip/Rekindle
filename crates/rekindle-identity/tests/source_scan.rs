//! Source-scan tests — structural invariants the type system cannot express.
//!
//! Each test reads source files as text and asserts/denies patterns.
//! A violation fails the build. These are the enforcement mechanism
//! for the seals documented in comments — without them, the seals
//! are aspirational.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Read all .rs files under a directory, returning (relative_path, contents) pairs.
fn read_rs_files(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    if !dir.exists() { return files; }
    for entry in walkdir(dir) {
        if entry.ends_with(".rs") {
            if let Ok(contents) = fs::read_to_string(&entry) {
                let rel = entry.strip_prefix(dir).unwrap_or(&entry);
                files.push((rel.display().to_string(), contents));
            }
        }
    }
    files
}

/// Simple recursive directory walk (no external dep).
fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                result.extend(walkdir(&path));
            } else {
                result.push(path);
            }
        }
    }
    result
}

fn src_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Check if a line is inside a #[cfg(test)] block.
/// Simple heuristic: track mod tests { ... } nesting.
fn is_in_test_block(contents: &str, target_line_idx: usize) -> bool {
    let mut in_test = false;
    let mut brace_depth: i32 = 0;
    let mut test_brace_start: i32 = 0;

    for (idx, line) in contents.lines().enumerate() {
        if idx == target_line_idx {
            return in_test;
        }
        if line.contains("#[cfg(test)]") {
            in_test = true;
            test_brace_start = brace_depth;
        }
        brace_depth += line.matches('{').count() as i32;
        brace_depth -= line.matches('}').count() as i32;
        if in_test && brace_depth <= test_brace_start {
            in_test = false;
        }
    }
    false
}

/// Find the enclosing function name for a given line index.
fn enclosing_fn(contents: &str, target_line_idx: usize) -> String {
    let lines: Vec<&str> = contents.lines().collect();
    for idx in (0..target_line_idx).rev() {
        let trimmed = lines[idx].trim();
        if trimmed.starts_with("pub fn ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub(crate) fn ")
        {
            if let Some(name_start) = trimmed.find("fn ") {
                let after_fn = &trimmed[name_start + 3..];
                let name_end = after_fn.find(|c: char| c == '(' || c == '<' || c == ' ')
                    .unwrap_or(after_fn.len());
                return after_fn[..name_end].to_string();
            }
        }
    }
    "<unknown>".to_string()
}

// ── PeerRef::anchor( allowlist — function-level ─────────────────

#[test]
fn anchor_mint_sites_are_allowlisted() {
    // PeerRef::anchor() must appear ONLY at sanctioned (file, function) pairs.
    // A new call site outside this allowlist is a seal breach.
    let allowed: BTreeSet<(&str, &str)> = [
        ("trust/store.rs", "observe_root_internal"),
        ("trust/store.rs", "import"),
        ("peer.rs", "bind"),
    ].into_iter().collect();

    let src = src_dir();
    let files = read_rs_files(&src);
    let mut violations = Vec::new();

    for (path, contents) in &files {
        for (line_idx, line) in contents.lines().enumerate() {
            if line.contains("PeerRef::anchor(") {
                if is_in_test_block(contents, line_idx) {
                    continue; // test code is exempt
                }
                let func = enclosing_fn(contents, line_idx);
                let matches = allowed.iter().any(|(f, fun)| {
                    path.ends_with(f) && func == *fun
                });
                if !matches {
                    violations.push(format!(
                        "{}::{}:{}: {}",
                        path, func, line_idx + 1, line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Unlisted PeerRef::anchor() call sites — seal breach:\n{}",
        violations.join("\n")
    );
}

// ── D-13: no persona import in enforcement modules ──────────────

#[test]
fn no_persona_import_in_enforcement_modules() {
    let enforcement_dirs = ["root", "grant", "trust", "session", "vault_label"];
    let src = src_dir();
    let mut violations = Vec::new();

    for dir_name in &enforcement_dirs {
        let dir = src.join(dir_name);
        for (path, contents) in read_rs_files(&dir) {
            for (line_idx, line) in contents.lines().enumerate() {
                if line.contains("crate::persona") || line.contains("super::persona") {
                    if is_in_test_block(&contents, line_idx) {
                        continue;
                    }
                    violations.push(format!(
                        "{}/{}:{}: {}",
                        dir_name, path, line_idx + 1, line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "D-13 violation: persona imported in enforcement module:\n{}",
        violations.join("\n")
    );
}

// ── No eprintln!/println! in production code ────────────────────

#[test]
fn no_eprintln_in_production_code() {
    let src = src_dir();
    let files = read_rs_files(&src);
    let mut violations = Vec::new();

    for (path, contents) in &files {
        for (line_idx, line) in contents.lines().enumerate() {
            if (line.contains("eprintln!") || line.contains("println!"))
                && !is_in_test_block(contents, line_idx)
            {
                violations.push(format!(
                    "{}:{}: {}",
                    path, line_idx + 1, line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "eprintln!/println! in production code (use tracing instead):\n{}",
        violations.join("\n")
    );
}

// ── Derivation tag uniqueness (source-level, not just runtime) ──

#[test]
fn derivation_tags_all_unique_and_namespaced() {
    let tags_file = src_dir().join("origin").join("tags.rs");
    let contents = fs::read_to_string(&tags_file)
        .expect("cannot read origin/tags.rs");

    let mut tag_values = Vec::new();

    for line in contents.lines() {
        // Only match actual constant declarations, not format strings or comments.
        if !line.contains("pub const") { continue; }
        if let Some(start) = line.find('"') {
            if let Some(end) = line[start + 1..].find('"') {
                let value = &line[start + 1..start + 1 + end];
                tag_values.push(value.to_string());
            }
        }
    }

    // All must be namespaced
    for tag in &tag_values {
        assert!(
            tag.starts_with("rekindle identity "),
            "derivation tag not namespaced: {tag}"
        );
    }

    // All must be unique
    let unique: BTreeSet<_> = tag_values.iter().collect();
    assert_eq!(
        unique.len(), tag_values.len(),
        "duplicate derivation tags found"
    );
}
