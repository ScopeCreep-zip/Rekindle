//! Configuration loading, validation, and precedence management.
//!
//! Provides the 8-layer config precedence system, policy enforcement,
//! and config inspection commands (`config show`, `config paths`, `config validate`).

pub mod loader;
pub mod schema;
pub mod validation;

pub use loader::load;
pub use schema::Config;
pub use validation::validate;

use crate::cli::ConfigCmd;
use crate::output::OutputMode;

/// Dispatch a `rekindle config` subcommand.
pub fn dispatch(cmd: &ConfigCmd, cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        ConfigCmd::Show => cmd_show(cfg, mode),
        ConfigCmd::Paths => cmd_paths(mode),
        ConfigCmd::Validate => cmd_validate(cfg, mode),
    }
}

/// `rekindle config show` — print the resolved config with all layers merged.
fn cmd_show(cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
    if mode.is_structured() {
        crate::output::format::print_structured(cfg, mode)
    } else {
        let toml_str = toml::to_string_pretty(cfg)
            .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
        crate::output::format::print_text(&toml_str)
    }
}

/// `rekindle config paths` — list all config file search paths in precedence order.
fn cmd_paths(mode: OutputMode) -> anyhow::Result<()> {
    let paths = loader::config_search_paths();
    let strings: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();

    if mode.is_structured() {
        crate::output::format::print_list(&strings, mode)
    } else {
        crate::output::format::print_text("Config file search paths (lowest → highest priority):")?;
        for (i, path) in paths.iter().enumerate() {
            let exists = if path.exists() { " (exists)" } else { "" };
            crate::output::format::print_text(&format!(
                "  {}. {}{}",
                i + 1,
                path.display(),
                exists
            ))?;
        }
        Ok(())
    }
}

/// `rekindle config validate` — validate config and report errors.
fn cmd_validate(cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
    match validate(cfg) {
        Ok(()) => {
            if mode.is_structured() {
                crate::output::format::print_structured(
                    &serde_json::json!({"valid": true, "errors": []}),
                    mode,
                )
            } else {
                crate::output::format::print_text("Config is valid.")
            }
        }
        Err(e) => {
            if mode.is_structured() {
                crate::output::format::print_structured(
                    &serde_json::json!({"valid": false, "errors": [e.to_string()]}),
                    mode,
                )
            } else {
                anyhow::bail!("config validation failed: {e}")
            }
        }
    }
}
