//! `check-allow-reasons` and `retrofit-allow-reasons`: every `#[allow]` has a reason.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::walk_source_files;

// ────────────────────────────────────────────────────────────────
// check-allow-reasons
// ────────────────────────────────────────────────────────────────
//
// Every `#[allow(...)]` (and `#![allow(...)]`) must include a
// `reason = "..."` argument. Bare allows are forbidden — see
// docs/contributor/ai-assisted-contributions.md §2.

pub(crate) fn check_allow_reasons(root: &Path) -> Result<()> {
    let bare_allow = find_bare_allows(root)?;
    if bare_allow.is_empty() {
        return Ok(());
    }
    eprintln!("Bare `#[allow(...)]` directives without `reason = \"…\"`:");
    let mut by_lint: BTreeMap<String, usize> = BTreeMap::new();
    for (path, lineno, lints) in &bare_allow {
        eprintln!("  • {}:{}  → {}", path.display(), lineno, lints.join(", "));
        for l in lints {
            *by_lint.entry(l.clone()).or_default() += 1;
        }
    }
    eprintln!("\nTotal: {} bare allow(s)", bare_allow.len());
    eprintln!("Top lints being silenced:");
    let mut top: Vec<_> = by_lint.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    for (lint, count) in top.into_iter().take(10) {
        eprintln!("  {count:>4}  {lint}");
    }
    eprintln!(
        "\nFix with `cargo xtask retrofit-allow-reasons` to add `reason = \"TODO: justify\"`\n\
         placeholders, then go through and write real reasons.\n\
         See docs/contributor/ai-assisted-contributions.md §2."
    );
    Err(anyhow!("{} bare allow(s)", bare_allow.len()))
}

fn find_bare_allows(root: &Path) -> Result<Vec<(PathBuf, usize, Vec<String>)>> {
    let mut out = Vec::new();
    for path in walk_source_files(root, &["rs"])? {
        let Ok(txt) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = txt.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let trimmed = lines[i].trim_start();
            if trimmed.starts_with("#[allow(") || trimmed.starts_with("#![allow(") {
                // An allow attribute may wrap across several lines under
                // rustfmt's vertical layout, so collect the whole
                // parenthesised span before deciding it is bare — a
                // `reason` on a continuation line still counts.
                let (span, end) = collect_attr_span(&lines, i);
                if !span.contains("reason") && !span.contains("nosemgrep") {
                    out.push((path.clone(), i + 1, extract_lints(&span)));
                }
                i = end + 1;
                continue;
            }
            i += 1;
        }
    }
    Ok(out)
}

/// Join lines from `start` until the `allow( … )` parenthesis closes,
/// returning the concatenated span text and the index of its final
/// line. Skips `//` line comments and `"…"` string literals so parens
/// inside a `reason` string or trailing comment don't skew the depth
/// count.
fn collect_attr_span(lines: &[&str], start: usize) -> (String, usize) {
    let mut span = String::new();
    let mut depth: i32 = 0;
    let mut started = false;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        if idx > start {
            span.push('\n');
        }
        span.push_str(line);
        let mut in_string = false;
        let mut escaped = false;
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '/' if chars.peek() == Some(&'/') => break,
                '"' => in_string = true,
                '(' => {
                    depth += 1;
                    started = true;
                }
                ')' => {
                    depth -= 1;
                    if started && depth == 0 {
                        return (span, idx);
                    }
                }
                _ => {}
            }
        }
    }
    (span, lines.len().saturating_sub(1))
}

fn extract_lints(line: &str) -> Vec<String> {
    let Some(start) = line.find('(').map(|i| i + 1) else {
        return Vec::new();
    };
    let Some(end) = line.rfind(')') else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    line[start..end]
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

// ────────────────────────────────────────────────────────────────
// retrofit-allow-reasons (one-shot migration helper)
// ────────────────────────────────────────────────────────────────
//
// Walks the workspace, finds every `#[allow(...)]` without a reason,
// and rewrites it to `#[allow(..., reason = "TODO: justify")]`. Run
// this once; then a contributor opens each file and replaces the
// TODO with a real reason.
pub(crate) fn retrofit_allow_reasons(root: &Path, dry_run: bool) -> Result<()> {
    let bare = find_bare_allows(root)?;
    if bare.is_empty() {
        println!("No bare allows found. Nothing to retrofit.");
        return Ok(());
    }

    // Group by file so we read once per file.
    let mut by_file: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (path, lineno, _) in &bare {
        by_file.entry(path.clone()).or_default().push(*lineno);
    }

    let mut total = 0usize;
    for (path, mut linenos) in by_file {
        linenos.sort_unstable();
        let original = std::fs::read_to_string(&path)?;
        let mut lines: Vec<String> = original.lines().map(ToOwned::to_owned).collect();
        for &lineno in &linenos {
            let idx = lineno - 1;
            if idx >= lines.len() {
                continue;
            }
            let line = &lines[idx];
            // Insert `, reason = "TODO: justify"` immediately before the
            // closing `)`, with a leading space if the lint list isn't
            // empty. Idempotent — skip if `reason` already present.
            if line.contains("reason") {
                continue;
            }
            if let Some(close) = line.rfind(')') {
                let mut new_line = line.clone();
                let needs_comma = !line[..close].trim_end().ends_with('(');
                let inject = if needs_comma {
                    ", reason = \"TODO: justify\""
                } else {
                    "reason = \"TODO: justify\""
                };
                new_line.insert_str(close, inject);
                lines[idx] = new_line;
                total += 1;
            }
        }
        let new_contents = lines.join("\n") + "\n";
        if dry_run {
            println!("would rewrite {} ({} lines)", path.display(), linenos.len());
        } else {
            std::fs::write(&path, new_contents)
                .with_context(|| format!("writing {}", path.display()))?;
            println!("rewrote {} ({} lines)", path.display(), linenos.len());
        }
    }

    println!(
        "\nTotal {} bare allow(s) {}.",
        total,
        if dry_run { "would be" } else { "were" }
    );
    println!(
        "Now open each modified file and replace `TODO: justify` with a real\n\
         justification, or refactor to remove the need for the allow."
    );
    Ok(())
}
