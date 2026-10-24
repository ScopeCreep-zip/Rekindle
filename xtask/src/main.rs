//! `cargo xtask` — workspace-wide checks that go beyond what cargo
//! natively offers.
//!
//! Subcommands:
//!
//!     cargo xtask check                Run the full guardrail bundle.
//!     cargo xtask check-boundaries     Crate-import tier boundaries.
//!     cargo xtask check-file-sizes     600-line ceiling, unconditional.
//!     cargo xtask check-allow-reasons  Every `#[allow(...)]` has reason="…".
//!     cargo xtask check-domain-literals
//!                                      Domain-separation labels only in
//!                                      rekindle-types::domains.
//!     cargo xtask check-no-emitter     src-tauri sends webview events only
//!                                      through event_dispatch (ADR 0007).
//!     cargo xtask check-capnp-bounds   Every Cap'n Proto read has a
//!                                      traversal bound (plan F4).
//!     cargo xtask check-veilid-dht-calls
//!                                      Veilid DHT/route calls only in their
//!                                      owners (record pool, RouteImports,
//!                                      OwnRoutes, relay offer).
//!     cargo xtask check-duplicate-constants
//!                                      One SCREAMING_CASE name must not hold
//!                                      different values in different crates.
//!     cargo xtask check-theme-literals Colours under src/styles are tokens.
//!     cargo xtask check-sec-surfaces   Security surfaces are unskinnable.
//!     cargo xtask check-destructive-controls
//!                                      Destructive controls carry a
//!                                      non-colour cue (rule D0).
//!     cargo xtask check-declared-grounds
//!                                      Every token-coloured text selector
//!                                      declares its ground.
//!     cargo xtask check-contrast-pairs Every declared pair reads 4.5:1
//!                                      (graphics 3:1) on each base.
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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use ignore::WalkBuilder;

mod allow_reasons;
mod bare_spawn;
mod boundaries;
mod capnp_bounds;
mod contrast_pairs;
mod css;
mod declared_grounds;
mod destructive_controls;
mod domain_literals;
mod duplicate_bodies;
mod duplicate_constants;
mod duplication;
mod file_sizes;
mod frontend_boundaries;
mod frontend_duplication;
mod intent_table;
mod jsx;
mod literals;
mod motion;
mod no_emitter;
mod no_managed_db;
mod pair_model;
mod platform_floor;
mod sec_styles;
mod sec_surfaces;
mod sqlite;
mod theme_literals;
mod veilid_boundary;
mod veilid_dht_calls;
mod wall_clock;

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
    /// Verify no frontend crate links the backend (veilid, transport, node…).
    CheckFrontendBoundaries,
    /// Verify the unconditional 600-line file-size ceiling.
    CheckFileSizes,
    /// Verify every `#[allow(...)]` has a `reason = "…"` argument.
    CheckAllowReasons,
    /// Verify no constant name holds different values in different crates.
    CheckDuplicateConstants,
    /// Verify domain-separation labels appear only in rekindle-types::domains.
    CheckDomainLiterals,
    /// Verify src-tauri never uses Tauri's `Emitter` (events go through the router).
    CheckNoEmitter,
    /// Verify every Cap'n Proto read has a traversal bound (no unbounded network decode).
    CheckCapnpBounds,
    /// Verify every Veilid DHT and private-route call stays in its owner.
    CheckVeilidDhtCalls,
    /// Verify SQL lives only in the storage crates (rekindle-db, -vault, -asql).
    CheckSqlite,
    /// Verify the database is reached through `AppState.db`, not Tauri managed state.
    CheckNoManagedDb,
    /// Verify session work spawns through a `SessionScope`, not bare `tokio::spawn`.
    CheckBareSpawn,
    /// Verify code the netsim runs reads no wall clock (`Instant::now`, `SystemTime::now`, …).
    CheckWallClock,
    /// Verify every command and IPC request is classified in the intent table.
    CheckIntentTable,
    /// Verify every colour under src/styles is a token (no hex, colour-function or named literal).
    CheckThemeLiterals,
    /// Verify security-critical elements use only the reserved `.sec-*` classes and `--sec-*` tokens.
    CheckSecSurfaces,
    /// Verify destructive controls have a visible verb label, are never default, one per view.
    CheckDestructiveControls,
    /// Verify every token-coloured text selector declares its ground in the region map.
    CheckDeclaredGrounds,
    /// Verify every declared text-on-ground pair reads 4.5:1 (graphics 3:1) on each base.
    CheckContrastPairs,
    /// Verify every transition reads a motion token and motion honours reduced motion.
    CheckMotion,
    /// Verify the macOS, WebKitGTK and Vite floors agree with the plan's E7.10 floor.
    CheckPlatformFloor,
    /// Verify built CSS keeps the declared `@layer` order (given the built files or dir).
    CheckBuiltCss { css: Vec<PathBuf> },
    /// Verify no function body is duplicated across crates.
    CheckDuplicateBodies,
    /// Verify no type name is declared in two crates.
    CheckDuplicateTypes,
    /// Verify no TS body or CSS rule is duplicated across files.
    CheckFrontendDuplication,
    /// Rewrite `xtask/known-duplication.txt` from the current tree.
    BaselineDuplication,
    /// Verify every workspace member inherits `[workspace.lints]`.
    CheckWorkspaceLints,
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
                    Box::new(|| boundaries::check_boundaries(&root))
                        as Box<dyn FnOnce() -> Result<()>>,
                ),
                (
                    "frontend-boundaries",
                    Box::new(|| frontend_boundaries::check_frontend_boundaries(&root)),
                ),
                (
                    "file-sizes",
                    Box::new(|| file_sizes::check_file_sizes(&root)),
                ),
                (
                    "allow-reasons",
                    Box::new(|| allow_reasons::check_allow_reasons(&root)),
                ),
                (
                    "duplicate-constants",
                    Box::new(|| duplicate_constants::check_duplicate_constants(&root)),
                ),
                (
                    "domain-literals",
                    Box::new(|| domain_literals::check_domain_literals(&root)),
                ),
                (
                    "no-emitter",
                    Box::new(|| no_emitter::check_no_emitter(&root)),
                ),
                (
                    "capnp-bounds",
                    Box::new(|| capnp_bounds::check_capnp_bounds(&root)),
                ),
                (
                    "veilid-dht-calls",
                    Box::new(|| veilid_dht_calls::check_veilid_dht_calls(&root)),
                ),
                ("sqlite", Box::new(|| sqlite::check_sqlite(&root))),
                (
                    "no-managed-db",
                    Box::new(|| no_managed_db::check_no_managed_db(&root)),
                ),
                (
                    "bare-spawn",
                    Box::new(|| bare_spawn::check_bare_spawn(&root)),
                ),
                (
                    "wall-clock",
                    Box::new(|| wall_clock::check_wall_clock(&root)),
                ),
                (
                    "intent-table",
                    Box::new(|| intent_table::check_intent_table(&root)),
                ),
                (
                    "theme-literals",
                    Box::new(|| theme_literals::check_theme_literals(&root)),
                ),
                (
                    "sec-surfaces",
                    Box::new(|| sec_surfaces::check_sec_surfaces(&root)),
                ),
                (
                    "destructive-controls",
                    Box::new(|| destructive_controls::check_destructive_controls(&root)),
                ),
                (
                    "declared-grounds",
                    Box::new(|| declared_grounds::check_declared_grounds(&root)),
                ),
                (
                    "contrast-pairs",
                    Box::new(|| contrast_pairs::check_contrast_pairs(&root)),
                ),
                ("motion", Box::new(|| motion::check_motion(&root))),
                (
                    "platform-floor",
                    Box::new(|| platform_floor::check_platform_floor(&root)),
                ),
                (
                    "duplicate-bodies",
                    Box::new(|| duplicate_bodies::check_duplicate_bodies(&root)),
                ),
                (
                    "duplicate-types",
                    Box::new(|| duplication::check_duplicate_types(&root)),
                ),
                (
                    "frontend-duplication",
                    Box::new(|| frontend_duplication::check_frontend_duplication(&root)),
                ),
                (
                    "workspace-lints",
                    Box::new(|| duplication::check_workspace_lints(&root)),
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
        Command::CheckBoundaries => boundaries::check_boundaries(&root),
        Command::CheckFrontendBoundaries => frontend_boundaries::check_frontend_boundaries(&root),
        Command::CheckFileSizes => file_sizes::check_file_sizes(&root),
        Command::CheckAllowReasons => allow_reasons::check_allow_reasons(&root),
        Command::CheckDuplicateConstants => duplicate_constants::check_duplicate_constants(&root),
        Command::CheckDomainLiterals => domain_literals::check_domain_literals(&root),
        Command::CheckNoEmitter => no_emitter::check_no_emitter(&root),
        Command::CheckCapnpBounds => capnp_bounds::check_capnp_bounds(&root),
        Command::CheckVeilidDhtCalls => veilid_dht_calls::check_veilid_dht_calls(&root),
        Command::CheckSqlite => sqlite::check_sqlite(&root),
        Command::CheckNoManagedDb => no_managed_db::check_no_managed_db(&root),
        Command::CheckBareSpawn => bare_spawn::check_bare_spawn(&root),
        Command::CheckWallClock => wall_clock::check_wall_clock(&root),
        Command::CheckIntentTable => intent_table::check_intent_table(&root),
        Command::CheckThemeLiterals => theme_literals::check_theme_literals(&root),
        Command::CheckSecSurfaces => sec_surfaces::check_sec_surfaces(&root),
        Command::CheckDestructiveControls => {
            destructive_controls::check_destructive_controls(&root)
        }
        Command::CheckDeclaredGrounds => declared_grounds::check_declared_grounds(&root),
        Command::CheckContrastPairs => contrast_pairs::check_contrast_pairs(&root),
        Command::CheckMotion => motion::check_motion(&root),
        Command::CheckPlatformFloor => platform_floor::check_platform_floor(&root),
        Command::CheckBuiltCss { css } => platform_floor::check_built_css(&root, css),
        Command::CheckDuplicateBodies => duplicate_bodies::check_duplicate_bodies(&root),
        Command::CheckDuplicateTypes => duplication::check_duplicate_types(&root),
        Command::CheckFrontendDuplication => {
            frontend_duplication::check_frontend_duplication(&root)
        }
        Command::BaselineDuplication => duplication::write_baseline(&root),
        Command::CheckWorkspaceLints => duplication::check_workspace_lints(&root),
        Command::RetrofitAllowReasons { dry_run } => {
            allow_reasons::retrofit_allow_reasons(&root, *dry_run)
        }
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

/// Walk `dir` for files with any of `exts`, honouring .gitignore and
/// skipping build/dependency trees.
///
/// `check_file_sizes` and `find_bare_allows` each hand-rolled this same
/// WalkBuilder + is_file + extension filter. (`check_boundaries` is a
/// flat, non-recursive `read_dir` over `crates/*` — a different shape,
/// deliberately left alone.)
pub(crate) fn walk_source_files(dir: &Path, exts: &[&str]) -> Result<Vec<PathBuf>> {
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
