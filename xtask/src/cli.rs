//! The command line `cargo xtask` parses: every subcommand and its
//! arguments (the crate root documents each one and dispatches them).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "Workspace-wide guardrail tasks for Rekindle",
    version,
    propagate_version = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) cmd: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Run every guardrail check (used by CI).
    Check,
    /// Verify crate-import tier boundaries (rekindle-secrets sole crypto, etc).
    CheckBoundaries,
    /// Run `cargo deny check` over advisories, bans, licenses and sources.
    CheckSupplyChain,
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
    /// Verify no IPC command takes a window region, shape or cursor-ignore flag (plan SK0 S4).
    CheckNoShapeIpc,
    /// Verify src/ declares no permission bit: the catalogue is the Host's (contract PN1).
    CheckNoShellPermissionBits,
    /// Verify every log sink and panic hook goes through `log_scrub` (rule B19).
    CheckLogScrub,
    /// Verify every Cap'n Proto read has a traversal bound (no unbounded network decode).
    CheckCapnpBounds,
    /// Verify key-material rows are deleted only through `rekindle_db::erase::delete`.
    CheckKeyErasure,
    /// Verify the WebDriver walk's driver cannot reach a release build.
    CheckWebdriverRelease,
    /// Verify every workspace library and binary compiles under the release profile.
    CheckRelease,
    /// Verify role ids are written only by the governance projection (no caches).
    CheckRoleIds,
    /// Verify every Veilid DHT and private-route call stays in its owner.
    CheckVeilidDhtCalls,
    /// Verify SQL lives only in the storage crates (rekindle-db, -vault, -asql).
    CheckSqlite,
    /// Verify the database is reached through `AppState.db`, not Tauri managed state.
    CheckNoManagedDb,
    /// Verify secret-shaped test literals come from rekindle-test-fixtures (owner 2026-11-26).
    CheckTestFixtures,
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
    /// Verify the isolation hook's command list matches invoke.rs and the capability grants.
    CheckIsolationCommands,
    /// Write the isolation hook's command list (`dist-isolation/commands.js`).
    GenIsolationCommands,
    /// Verify the skin registry's generated files are current and its defaults meet their constraints.
    CheckSkinRegistry,
    /// Write the skin registry's generated CSS, TS, TUI palettes and JSON Schema.
    GenSkinRegistry,
    /// Write the skin validator's pair model from the sheets and `src/styles/regions.toml`.
    GenContrastPairs,
    /// Print the silhouette a built-in skin cuts a window to (the shaper's own computation), as JSON rects in device pixels.
    SkinSilhouette(crate::skin_silhouette::Args),
    /// Import a legacy skin tree as the built-in pack `crates/rekindle-skin/builtin/<id>.rkskin`.
    ImportSkin {
        #[arg(long, value_enum)]
        format: crate::import_skin::Format,
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        builtin: String,
        /// The page stating Xfire made the skin (with `--credit-quote`).
        #[arg(long, requires = "credit_quote")]
        credit_source: Option<String>,
        /// That page's words.
        #[arg(long, requires = "credit_source")]
        credit_quote: Option<String>,
        /// A line our copy of the pack source corrects (file, line, before →
        /// after, why), recorded in the NOTICE; repeatable.
        #[arg(long)]
        source_correction: Vec<String>,
        /// Import and report the outcome without writing the pack.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write the Xfire game icons, game id mapping and sound banks from the owner's tree.
    ImportXfireData {
        #[arg(long)]
        from: PathBuf,
    },
    /// Verify every `ICON_*` codepoint is in the bundled font and is the glyph its comment names.
    CheckIconGlyphs,
    /// Verify every transition reads a motion token and motion honours reduced motion.
    CheckMotion,
    /// Verify the U1 review's z-layer is above every other overlay layer.
    CheckOverlayLayers,
    /// Verify every window throttles in the background and none is suspended.
    CheckBackgroundThrottling,
    /// Verify the macOS, WebKitGTK and Vite floors agree with the plan's E7.10 floor.
    CheckPlatformFloor,
    /// Verify every release bundle declares, stages and ships the skin helper.
    CheckSidecars,
    /// Build the skin helper and stage it under `src-tauri/binaries/` for `externalBin`.
    StageSidecar {
        /// The target triple; defaults to the Tauri CLI's `TAURI_ENV_TARGET_TRIPLE`.
        #[arg(long)]
        target: Option<String>,
    },
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
    /// Verify no inline `mod name { … }` (bodies live in `<stem>_<name>.rs` files).
    CheckInlineModules,
    /// Verify the peer-input crates deny the panic-class clippy lints.
    CheckPanicLints,
    /// Verify secret types never derive Debug or format instance data.
    CheckSecretDebug,
    /// Move each inline module body to `<stem>_<name>.rs` beside its parent.
    OutlineModules {
        /// Only this workspace member (package name).
        #[arg(long = "crate")]
        krate: Option<String>,
    },
    /// Run the `#[ignore]`d wall-clock reports in release (by hand; not part of `check`).
    Perf,
    /// One-shot helper: add `reason = "TODO: justify"` to bare allows.
    RetrofitAllowReasons {
        /// Print what would change without writing files.
        #[arg(long)]
        dry_run: bool,
    },
}
