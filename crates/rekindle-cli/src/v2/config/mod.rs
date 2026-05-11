//! Configuration loading, validation, and precedence management.
//!
//! 8-layer config precedence system with policy enforcement.

pub mod loader;
pub mod schema;
pub mod validation;

pub use loader::load;
pub use schema::Config;
pub use validation::validate;

use crate::v2::cli::ConfigCmd;
use crate::v2::output::OutputMode;

/// Dispatch a `rekindle config` subcommand.
pub fn dispatch(cmd: &ConfigCmd, cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        ConfigCmd::Show => cmd_show(cfg, mode),
        ConfigCmd::Paths => cmd_paths(mode),
        ConfigCmd::Validate => cmd_validate(cfg, mode),
    }
}

fn cmd_show(cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
    if mode.is_structured() {
        crate::v2::output::format::print_structured(cfg, mode)
    } else {
        let toml_str = toml::to_string_pretty(cfg)
            .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
        crate::v2::output::format::print_text(&toml_str)
    }
}

fn cmd_paths(mode: OutputMode) -> anyhow::Result<()> {
    let paths = loader::config_search_paths();
    let strings: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();

    if mode.is_structured() {
        crate::v2::output::format::print_list(&strings, mode)
    } else {
        crate::v2::output::format::print_text("Config file search paths (lowest → highest priority):")?;
        for (i, path) in paths.iter().enumerate() {
            let exists = if path.exists() { " (exists)" } else { "" };
            crate::v2::output::format::print_text(&format!(
                "  {}. {}{exists}",
                i + 1,
                path.display(),
            ))?;
        }
        Ok(())
    }
}

fn cmd_validate(cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
    match validate(cfg) {
        Ok(()) => {
            if mode.is_structured() {
                crate::v2::output::format::print_structured(
                    &serde_json::json!({"valid": true, "errors": []}),
                    mode,
                )
            } else {
                crate::v2::output::format::print_text("Config is valid.")
            }
        }
        Err(e) => {
            if mode.is_structured() {
                crate::v2::output::format::print_structured(
                    &serde_json::json!({"valid": false, "errors": [e.to_string()]}),
                    mode,
                )
            } else {
                anyhow::bail!("config validation failed: {e}")
            }
        }
    }
}
