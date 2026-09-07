//! `cargo xtask` — workspace-wide checks that go beyond what cargo
//! natively offers.
//!
//! Subcommands:
//!
//!     cargo xtask check                Run the full guardrail bundle.
//!     cargo xtask check-boundaries     Crate-import tier boundaries.
//!     cargo xtask check-file-sizes     600-line ceiling, unconditional.
//!     cargo xtask check-allow-reasons  Every `#[allow(...)]` has reason="…".
//!     cargo xtask check-duplicate-constants
//!                                      One SCREAMING_CASE name must not hold
//!                                      different values in different crates.
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
    clippy::print_stderr,
    reason = "xtask is a CLI; printing to the terminal is its product, not stray debug output (clippy treats print_stdout/print_stderr as restriction lints to exempt CLI binaries from)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use ignore::WalkBuilder;
use quote::ToTokens as _;

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
    /// Verify the unconditional 600-line file-size ceiling.
    CheckFileSizes,
    /// Verify every `#[allow(...)]` has a `reason = "…"` argument.
    CheckAllowReasons,
    /// Verify no constant name holds different values in different crates.
    CheckDuplicateConstants,
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
                (
                    "duplicate-constants",
                    Box::new(|| check_duplicate_constants(&root)),
                ),
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
        Command::CheckDuplicateConstants => check_duplicate_constants(&root),
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
    // ── Decomposed-harvest plan (memoized-dazzling-torvalds.md) ──
    // Phase 2: SQLCipher + per-entry AES-256-GCM vault layer.
    "rekindle-vault",
    // Phase 4: BLAKE3 keyed hash chain. (BLAKE3 is not in CRYPTO_CRATES,
    // but listing here keeps the intent explicit if future deps grow.)
    "rekindle-audit",
    // Phase 17: cascade MEK rotation — wraps MEKs via rekindle-crypto's
    // mek_distribution helpers.
    "rekindle-mek-rotation",
    // Phase 18: community lifecycle orchestration (origin / bootstrap /
    // join / segments / apply) — uses rekindle-crypto for pseudonym
    // derivation + governance entry signing.
    "rekindle-governance-runtime",
    // Phase 19: community channel messaging (send / receive / threads /
    // reactions / expressions / mentions) — uses rekindle-crypto for
    // MEK encrypt/decrypt + signed-envelope auth.
    "rekindle-channel",
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
        if dep_present(&toml, "veilid-core") && !VEILID_ALLOWED.contains(&crate_name.as_str()) {
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

/// Walk `dir` for files with any of `exts`, honouring .gitignore and
/// skipping build/dependency trees.
///
/// `check_file_sizes` and `find_bare_allows` each hand-rolled this same
/// WalkBuilder + is_file + extension filter. (`check_boundaries` is a
/// flat, non-recursive `read_dir` over `crates/*` — a different shape,
/// deliberately left alone.)
fn walk_source_files(dir: &Path, exts: &[&str]) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let walker = WalkBuilder::new(dir)
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
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(path.to_path_buf());
        }
    }
    Ok(out)
}

/// One ceiling for every source file, Rust and frontend alike. Any
/// file over it fails the gate — no allowlist, no escape hatch. A
/// deliberate future exception would require editing this check in a
/// reviewed PR, which is exactly the friction an exception deserves.
const MAX_LINES: usize = 600;

fn check_file_sizes(root: &Path) -> Result<()> {
    let mut offenders = 0usize;

    // `css` is in here because a 7,926-line xfire-theme.css was the one
    // source file the ceiling could not see. It is now split under
    // src/styles/theme/, and this keeps it that way.
    for (subdir, exts) in [
        ("src", &["ts", "tsx", "css"][..]),
        ("src-tauri/src", &["rs"][..]),
        ("crates", &["rs"][..]),
    ] {
        for path in walk_source_files(&root.join(subdir), exts)? {
            let lines = std::fs::read_to_string(&path)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            if lines <= MAX_LINES {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            println!("  ✗ {rel} ({lines} lines, ceiling {MAX_LINES})");
            offenders += 1;
        }
    }

    if offenders > 0 {
        return Err(anyhow!(
            "{offenders} file(s) over {MAX_LINES} lines.\n\
             Split the file along a module seam (see docs/contributor/architecture-rules.md)."
        ));
    }
    Ok(())
}

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
        "MIGRATION",
        "`include_str!` of the same 001_init.sql; the literals differ only \
         by relative depth from the embedding file.",
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
fn crate_of(rel_path: &str) -> String {
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

fn check_duplicate_constants(root: &Path) -> Result<()> {
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
