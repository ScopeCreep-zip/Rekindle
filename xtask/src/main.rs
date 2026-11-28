//! `cargo xtask` — workspace-wide checks that go beyond what cargo
//! natively offers.
//!
//! Subcommands:
//!
//!     cargo xtask check                Run the full guardrail bundle.
//!     cargo xtask check-boundaries     Crate-import tier boundaries.
//!     cargo xtask check-supply-chain   `cargo deny check` (advisories, bans,
//!                                      licenses, sources; deny.toml).
//!     cargo xtask check-file-sizes     600-line ceiling, unconditional.
//!     cargo xtask check-allow-reasons  Every `#[allow(...)]` has reason="…".
//!     cargo xtask check-domain-literals
//!                                      Domain-separation labels only in
//!                                      rekindle-types::domains.
//!     cargo xtask check-no-emitter     src-tauri sends webview events only
//!                                      through event_dispatch (ADR 0007).
//!     cargo xtask check-no-shape-ipc  No IPC command takes a window region
//!                                      or a cursor-ignore flag (SK0 S4).
//!     cargo xtask check-no-shell-permission-bits
//!                                      src/ declares no permission bit: the
//!                                      catalogue is the Host's (PN1).
//!     cargo xtask check-log-scrub      Every tracing subscriber writes through
//!                                      ScrubbingMakeWriter; every binary installs the scrubbing panic hook (B19).
//!     cargo xtask check-capnp-bounds   Every Cap'n Proto read has a
//!                                      traversal bound (plan F4).
//!     cargo xtask check-key-erasure    Key-material rows are deleted only
//!                                      through rekindle_db::erase (I14).
//!     cargo xtask check-webdriver-release
//!                                      The WebDriver driver never builds in release.
//!     cargo xtask check-release        The workspace compiles in release (REL1).
//!     cargo xtask check-role-ids       Role ids are written only by the
//!                                      governance projection (RV7 M1).
//!     cargo xtask check-veilid-dht-calls
//!                                      Veilid DHT/route calls only in their
//!                                      owners (record pool, RouteImports,
//!                                      OwnRoutes, relay offer).
//!     cargo xtask check-duplicate-constants
//!                                      One SCREAMING_CASE name must not hold
//!                                      different values in different crates.
//!     cargo xtask check-theme-literals Colours under src/styles are tokens.
//!     cargo xtask check-sec-surfaces   Security surfaces are unskinnable.
//!     cargo xtask check-overlay-layers The U1 review is the top overlay layer.
//!     cargo xtask check-background-throttling
//!                                      Every window throttles in the
//!                                      background, none suspends (A1.5).
//!     cargo xtask check-destructive-controls
//!                                      Destructive controls carry a
//!                                      non-colour cue (rule D0).
//!     cargo xtask check-declared-grounds
//!                                      Every token-coloured text selector
//!                                      declares its ground.
//!     cargo xtask check-contrast-pairs Every declared pair reads 4.5:1
//!                                      (graphics 3:1) on each base.
//!     cargo xtask check-isolation-commands
//!                                      The isolation hook's command list is
//!                                      current (plan E7.1, roadmap 3.3).
//!     cargo xtask gen-isolation-commands
//!                                      Write dist-isolation/commands.js from
//!                                      invoke.rs and the capability grants.
//!     cargo xtask check-skin-registry  The skin registry's generated files
//!                                      are current (plan E7.7.1).
//!     cargo xtask check-sidecars       The skin helper ships beside the
//!                                      bundled app (plan E7.7.1, F3).
//!     cargo xtask stage-sidecar [--target <triple>]
//!                                      Build and stage the skin helper as
//!                                      Tauri's externalBin expects.
//!     cargo xtask gen-skin-registry    Write the skin registry's generated
//!                                      CSS, TS, TUI palettes and schema.
//!     cargo xtask import-skin --format xfire-xml --from <dir> --builtin <id>
//!     cargo xtask import-skin --format xfire-ini --from <dir> --builtin <id>
//!     cargo xtask import-skin --format pidgin --from <theme dir|.tgz> --builtin <id>
//!     cargo xtask import-skin --format miranda --from <skin dir> --builtin <id>
//!     cargo xtask import-skin --format mumble --from <theme dir> --builtin <id>
//!     cargo xtask import-skin --format teamspeak3 --from <style dir> --builtin <id>
//!                                      Import a legacy skin as a built-in
//!                                      pack and its NOTICE (plan E7.7.3).
//!         [--credit-source <url> --credit-quote <text>] [--source-correction <text>]...
//!                                      Xfire's authorship stated outside the
//!                                      skin's files (xfire-xml/xfire-ini).
//!         [--dry-run]                  Import and report; write nothing.
//!     cargo xtask check-icon-glyphs    Every ICON_* codepoint is the glyph
//!                                      its comment names in the bundled
//!                                      Nerd Font (plan E7.12).
//!     cargo xtask import-xfire-data --from <legacy/unpacked>
//!                                      Write the Xfire game icons, the game
//!                                      id mapping and the built-in sound
//!                                      banks (plan E7.7.3).
//!     cargo xtask check-inline-modules No inline `mod name { … }` (gate item 1).
//!     cargo xtask check-panic-lints    Peer-input crates deny the panic lints (gate item 3(a)).
//!     cargo xtask check-secret-debug  Secret types keep fields out of Debug (gate item 3(d)).
//!     cargo xtask outline-modules [--crate <name>]
//!                                      Move each inline module body to
//!                                      `<stem>_<name>.rs` (the fixer).
//!     cargo xtask perf                 Run the ignored wall-clock reports in
//!                                      release, by hand; never in `check`.
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

use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::Parser;
use cli::{Cli, Command};
pub(crate) use walk::walk_source_files;
use walk::workspace_root;

mod allow_reasons;
mod background_throttling;
mod bare_spawn;
mod boundaries;
mod capnp_bounds;
mod cargo_meta;
mod cli;
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
mod gen_isolation_commands;
mod gen_skin_registry;
mod icon_glyphs;
mod import_skin;
mod import_xfire_data;
mod inline_modules;
mod intent_table;
mod jsx;
mod key_erasure;
mod literals;
mod log_scrub;
mod motion;
mod no_emitter;
mod no_managed_db;
mod no_shape_ipc;
mod outline_modules;
mod overlay_layers;
mod pair_model;
mod panic_lints;
mod perf;
mod platform_floor;
mod release_build;
mod role_ids;
mod rust_lex;
mod rust_uses;
mod sec_styles;
mod sec_surfaces;
mod secret_debug;
mod shell_permission_bits;
mod sidecars;
mod skin_silhouette;
mod sqlite;
mod supply_chain;
mod test_code;
mod test_fixtures;
mod theme_literals;
mod veilid_boundary;
mod veilid_dht_calls;
mod walk;
mod wall_clock;
mod webdriver_release;
mod woff2;

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
                    "inline-modules",
                    Box::new(|| inline_modules::check_inline_modules(&root)),
                ),
                (
                    "panic-lints",
                    Box::new(|| panic_lints::check_panic_lints(&root)),
                ),
                ("secret-debug", Box::new(|| secret_debug::check(&root))),
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
                    "no-shape-ipc",
                    Box::new(|| no_shape_ipc::check_no_shape_ipc(&root)),
                ),
                (
                    "no-shell-permission-bits",
                    Box::new(|| shell_permission_bits::check_no_shell_permission_bits(&root)),
                ),
                ("log-scrub", Box::new(|| log_scrub::check_log_scrub(&root))),
                ("role-ids", Box::new(|| role_ids::check_role_ids(&root))),
                (
                    "capnp-bounds",
                    Box::new(|| capnp_bounds::check_capnp_bounds(&root)),
                ),
                (
                    "key-erasure",
                    Box::new(|| key_erasure::check_key_erasure(&root)),
                ),
                (
                    "webdriver-release",
                    Box::new(|| webdriver_release::check_webdriver_release(&root)),
                ),
                ("release", Box::new(|| release_build::check_release(&root))),
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
                    "test-fixtures",
                    Box::new(|| test_fixtures::check_test_fixtures(&root)),
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
                (
                    "isolation-commands",
                    Box::new(|| gen_isolation_commands::check_isolation_commands(&root)),
                ),
                (
                    "skin-registry",
                    Box::new(|| gen_skin_registry::check_skin_registry(&root)),
                ),
                ("motion", Box::new(|| motion::check_motion(&root))),
                (
                    "overlay-layers",
                    Box::new(|| overlay_layers::check_overlay_layers(&root)),
                ),
                (
                    "icon-glyphs",
                    Box::new(|| icon_glyphs::check_icon_glyphs(&root)),
                ),
                (
                    "background-throttling",
                    Box::new(|| background_throttling::check_background_throttling(&root)),
                ),
                (
                    "platform-floor",
                    Box::new(|| platform_floor::check_platform_floor(&root)),
                ),
                ("sidecars", Box::new(|| sidecars::check_sidecars(&root))),
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
                (
                    "supply-chain",
                    Box::new(|| supply_chain::check_supply_chain(&root)),
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
        Command::CheckSupplyChain => supply_chain::check_supply_chain(&root),
        Command::CheckFrontendBoundaries => frontend_boundaries::check_frontend_boundaries(&root),
        Command::CheckFileSizes => file_sizes::check_file_sizes(&root),
        Command::CheckAllowReasons => allow_reasons::check_allow_reasons(&root),
        Command::CheckDuplicateConstants => duplicate_constants::check_duplicate_constants(&root),
        Command::CheckDomainLiterals => domain_literals::check_domain_literals(&root),
        Command::CheckNoEmitter => no_emitter::check_no_emitter(&root),
        Command::CheckNoShapeIpc => no_shape_ipc::check_no_shape_ipc(&root),
        Command::CheckNoShellPermissionBits => {
            shell_permission_bits::check_no_shell_permission_bits(&root)
        }
        Command::CheckLogScrub => log_scrub::check_log_scrub(&root),
        Command::CheckCapnpBounds => capnp_bounds::check_capnp_bounds(&root),
        Command::CheckKeyErasure => key_erasure::check_key_erasure(&root),
        Command::CheckWebdriverRelease => webdriver_release::check_webdriver_release(&root),
        Command::CheckRelease => release_build::check_release(&root),
        Command::CheckRoleIds => role_ids::check_role_ids(&root),
        Command::CheckVeilidDhtCalls => veilid_dht_calls::check_veilid_dht_calls(&root),
        Command::CheckSqlite => sqlite::check_sqlite(&root),
        Command::CheckNoManagedDb => no_managed_db::check_no_managed_db(&root),
        Command::CheckTestFixtures => test_fixtures::check_test_fixtures(&root),
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
        Command::CheckIsolationCommands => gen_isolation_commands::check_isolation_commands(&root),
        Command::GenIsolationCommands => gen_isolation_commands::gen_isolation_commands(&root),
        Command::CheckSkinRegistry => gen_skin_registry::check_skin_registry(&root),
        Command::GenSkinRegistry => gen_skin_registry::gen_skin_registry(&root),
        Command::GenContrastPairs => contrast_pairs::gen_contrast_pairs(&root),
        Command::SkinSilhouette(args) => skin_silhouette::skin_silhouette(args),
        Command::ImportSkin {
            format,
            from,
            builtin,
            credit_source,
            credit_quote,
            source_correction,
            dry_run,
        } => import_skin::import_skin(
            &root,
            &import_skin::Request {
                format: *format,
                from,
                id: builtin,
                credit: credit_source.as_deref().zip(credit_quote.as_deref()),
                corrections: source_correction,
                dry_run: *dry_run,
            },
        ),
        Command::ImportXfireData { from } => import_xfire_data::import_xfire_data(&root, from),
        Command::CheckMotion => motion::check_motion(&root),
        Command::CheckOverlayLayers => overlay_layers::check_overlay_layers(&root),
        Command::CheckBackgroundThrottling => {
            background_throttling::check_background_throttling(&root)
        }
        Command::CheckIconGlyphs => icon_glyphs::check_icon_glyphs(&root),
        Command::CheckPlatformFloor => platform_floor::check_platform_floor(&root),
        Command::CheckSidecars => sidecars::check_sidecars(&root),
        Command::StageSidecar { target } => sidecars::stage_sidecar(&root, target.as_deref()),
        Command::CheckBuiltCss { css } => platform_floor::check_built_css(&root, css),
        Command::CheckDuplicateBodies => duplicate_bodies::check_duplicate_bodies(&root),
        Command::CheckDuplicateTypes => duplication::check_duplicate_types(&root),
        Command::CheckFrontendDuplication => {
            frontend_duplication::check_frontend_duplication(&root)
        }
        Command::BaselineDuplication => duplication::write_baseline(&root),
        Command::CheckWorkspaceLints => duplication::check_workspace_lints(&root),
        Command::CheckInlineModules => inline_modules::check_inline_modules(&root),
        Command::CheckPanicLints => panic_lints::check_panic_lints(&root),
        Command::CheckSecretDebug => secret_debug::check(&root),
        Command::OutlineModules { krate } => {
            outline_modules::outline_modules(&root, krate.as_deref())
        }
        Command::Perf => perf::perf(&root),
        Command::RetrofitAllowReasons { dry_run } => {
            allow_reasons::retrofit_allow_reasons(&root, *dry_run)
        }
    }
}
