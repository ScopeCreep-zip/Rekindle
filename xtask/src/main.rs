//! `cargo xtask` — workspace-wide checks that go beyond what cargo
//! natively offers.
//!
//! Subcommands:
//!
//!     cargo xtask check                Run the full guardrail bundle.
//!     cargo xtask check-boundaries     Crate-import tier boundaries.
//!     cargo xtask check-file-sizes     File-size thresholds.
//!     cargo xtask check-allow-reasons  Every `#[allow(...)]` has reason="…".
//!     cargo xtask retrofit-allow-reasons
//!                                      Add `reason = "TODO: justify"`
//!                                      placeholders to existing allows
//!                                      (one-shot migration helper).
//!
//! See `docs/contributor/architecture-rules.md` for the binding tier
//! hierarchy and `docs/contributor/ai-assisted-contributions.md` for
//! the rollout schedule for each gate.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    reason = "xtask is a CLI; structured output is human-read only"
)]
#![allow(
    clippy::print_stderr,
    reason = "xtask is a CLI; structured output is human-read only"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use ignore::WalkBuilder;

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "Workspace-wide guardrail tasks for Rekindle",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run every guardrail check (used by CI).
    Check,
    /// Verify crate-import tier boundaries (rekindle-secrets sole crypto, etc).
    CheckBoundaries,
    /// Verify file-size thresholds (warn-only for now).
    CheckFileSizes,
    /// Verify every `#[allow(...)]` has a `reason = "…"` argument.
    CheckAllowReasons,
    /// One-shot helper: add `reason = "TODO: justify"` to bare allows.
    RetrofitAllowReasons {
        /// Print what would change without writing files.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(&cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn dispatch(cmd: &Command) -> Result<()> {
    let root = workspace_root()?;
    match cmd {
        Command::Check => {
            // Run every check; collect failures and report at the end.
            let mut failures = 0u32;
            for (label, runner) in [
                (
                    "boundaries",
                    Box::new(|| check_boundaries(&root)) as Box<dyn FnOnce() -> Result<()>>,
                ),
                ("file-sizes", Box::new(|| check_file_sizes(&root))),
                ("allow-reasons", Box::new(|| check_allow_reasons(&root))),
            ] {
                println!("\n── xtask: {label}");
                if let Err(e) = runner() {
                    eprintln!("    FAILED: {e:#}");
                    failures += 1;
                } else {
                    println!("    OK");
                }
            }
            if failures > 0 {
                return Err(anyhow!("{failures} check(s) failed"));
            }
            Ok(())
        }
        Command::CheckBoundaries => check_boundaries(&root),
        Command::CheckFileSizes => check_file_sizes(&root),
        Command::CheckAllowReasons => check_allow_reasons(&root),
        Command::RetrofitAllowReasons { dry_run } => retrofit_allow_reasons(&root, *dry_run),
    }
}

fn workspace_root() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR is set when invoked via `cargo run -p xtask`.
    // Walk up until we find the workspace Cargo.toml.
    let mut p = std::env::current_dir().context("getting current dir")?;
    loop {
        let cargo = p.join("Cargo.toml");
        if cargo.exists() {
            let txt = std::fs::read_to_string(&cargo)?;
            if txt.contains("[workspace]") {
                return Ok(p);
            }
        }
        if !p.pop() {
            return Err(anyhow!("could not find workspace root"));
        }
    }
}

// ────────────────────────────────────────────────────────────────
// check-boundaries
// ────────────────────────────────────────────────────────────────
//
// Tier-2 invariant: only `rekindle-secrets` may import these crypto
// crates directly. Tier-3+ consumers must go through `rekindle-secrets`.
const CRYPTO_CRATES: &[&str] = &[
    "ed25519-dalek",
    "x25519-dalek",
    "aes-gcm",
    "chacha20poly1305",
    "hkdf",
];
// Veilid integration is centralised: only the daemon-track transport
// or the desktop-track protocol crate may import veilid-core directly.
const VEILID_ALLOWED: &[&str] = &["rekindle-transport", "rekindle-protocol"];
// Crypto-allowed crates for now; this list will shrink as the cleanup
// sweep refactors crypto consumers to consume via rekindle-secrets.
const CRYPTO_ALLOWED: &[&str] = &[
    "rekindle-secrets",
    // ── Pending sweep — see ai-assisted-contributions.md §5 ──
    "rekindle-crypto",
    "rekindle-dm",
    "rekindle-calls",
    "rekindle-voice",
    "rekindle-node",
    "rekindle-transport",
    "rekindle-records",
    "rekindle-route",
    "rekindle-sync",
    "rekindle-protocol",
];

fn check_boundaries(root: &Path) -> Result<()> {
    let crates_dir = root.join("crates");
    if !crates_dir.exists() {
        return Err(anyhow!("crates/ directory not found"));
    }

    let mut violations: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&crates_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let crate_name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| anyhow!("non-utf8 crate dir"))?
            .to_owned();
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let toml = std::fs::read_to_string(&manifest)?;

        for crypto in CRYPTO_CRATES {
            if dep_present(&toml, crypto) && !CRYPTO_ALLOWED.contains(&crate_name.as_str()) {
                violations.push(format!(
                    "{crate_name}: imports `{crypto}` (Tier 2 crypto boundary — only via rekindle-secrets)"
                ));
            }
        }
        if dep_present(&toml, "veilid-core")
            && !VEILID_ALLOWED.contains(&crate_name.as_str())
        {
            violations.push(format!(
                "{crate_name}: imports `veilid-core` (Veilid boundary — only via rekindle-transport / rekindle-protocol)"
            ));
        }
    }

    if violations.is_empty() {
        return Ok(());
    }
    eprintln!("Tier-boundary violations:");
    for v in &violations {
        eprintln!("  • {v}");
    }
    eprintln!(
        "\nSee docs/contributor/architecture-rules.md for the binding hierarchy.\n\
         Most existing violations are tracked in the cleanup sweep — see\n\
         docs/contributor/ai-assisted-contributions.md §5."
    );
    Err(anyhow!("{} boundary violation(s)", violations.len()))
}

fn dep_present(toml: &str, dep: &str) -> bool {
    // Crude but adequate: a `name = "..."` table-form entry or a
    // `name.workspace = true` line. Avoids pulling in toml/serde here
    // to keep xtask compile time cheap.
    let needles = [
        format!("\n{dep} = "),
        format!("\n{dep}.workspace"),
        format!("\n\"{dep}\" = "),
        format!("name = \"{dep}\""),
    ];
    needles.iter().any(|n| toml.contains(n.as_str()))
}

// ────────────────────────────────────────────────────────────────
// check-file-sizes
// ────────────────────────────────────────────────────────────────
//
// Soft thresholds — emit warnings; CI gate decides whether to fail.
// Tighter limits will land once the existing oversized files are split.
const FRONTEND_MAX_LINES: usize = 500;
const RUST_MAX_LINES: usize = 1500;

fn check_file_sizes(root: &Path) -> Result<()> {
    let mut warnings = 0usize;

    for (subdir, ext, threshold) in [
        ("src", &["ts", "tsx"][..], FRONTEND_MAX_LINES),
        ("src-tauri/src", &["rs"][..], RUST_MAX_LINES),
        ("crates", &["rs"][..], RUST_MAX_LINES),
    ] {
        let dir = root.join(subdir);
        if !dir.exists() {
            continue;
        }
        let walker = WalkBuilder::new(&dir)
            .standard_filters(true)
            .build();
        for entry in walker {
            let entry = entry?;
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let Some(file_ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !ext.contains(&file_ext) {
                continue;
            }
            let lines = std::fs::read_to_string(path)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            if lines > threshold {
                println!(
                    "  ⚠ {} ({} lines, threshold {})",
                    path.strip_prefix(root).unwrap_or(path).display(),
                    lines,
                    threshold
                );
                warnings += 1;
            }
        }
    }

    if warnings > 0 {
        println!(
            "\n{warnings} file(s) over threshold. These are tracked in the cleanup sweep —\n\
             new files added in a PR should stay under threshold."
        );
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────
// check-allow-reasons
// ────────────────────────────────────────────────────────────────
//
// Every `#[allow(...)]` (and `#![allow(...)]`) must include a
// `reason = "..."` argument. Bare allows are forbidden — see
// docs/contributor/ai-assisted-contributions.md §2.

fn check_allow_reasons(root: &Path) -> Result<()> {
    let bare_allow = find_bare_allows(root)?;
    if bare_allow.is_empty() {
        return Ok(());
    }
    eprintln!("Bare `#[allow(...)]` directives without `reason = \"…\"`:");
    let mut by_lint: BTreeMap<String, usize> = BTreeMap::new();
    for (path, lineno, lints) in &bare_allow {
        eprintln!(
            "  • {}:{}  → {}",
            path.display(),
            lineno,
            lints.join(", ")
        );
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
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .filter_entry(|e| {
            !e.path()
                .components()
                .any(|c| matches!(c.as_os_str().to_str(), Some("target" | "node_modules")))
        })
        .build();
    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(txt) = std::fs::read_to_string(path) else {
            continue;
        };
        for (lineno, line) in txt.lines().enumerate() {
            let trimmed = line.trim_start();
            if (trimmed.starts_with("#[allow(") || trimmed.starts_with("#![allow("))
                && !line.contains("reason")
                && !line.contains("nosemgrep")
            {
                let lints = extract_lints(line);
                out.push((path.to_owned(), lineno + 1, lints));
            }
        }
    }
    Ok(out)
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
fn retrofit_allow_reasons(root: &Path, dry_run: bool) -> Result<()> {
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

    println!("\nTotal {} bare allow(s) {}.", total, if dry_run { "would be" } else { "were" });
    println!(
        "Now open each modified file and replace `TODO: justify` with a real\n\
         justification, or refactor to remove the need for the allow."
    );
    Ok(())
}
