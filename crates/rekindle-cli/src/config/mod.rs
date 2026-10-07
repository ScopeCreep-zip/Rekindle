//! `rekindle config`: inspect the layered `config.toml` that `rekindled`
//! and the TUI read (`rekindle_client::config`, one schema for all).

use std::path::Path;

use rekindle_client::config::{self, Layer};

use crate::cli::ConfigCmd;
use crate::output::{format, OutputMode};

/// Dispatch a `rekindle config` subcommand. `explicit` is `--config`.
pub fn dispatch(cmd: &ConfigCmd, explicit: Option<&Path>, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        ConfigCmd::Show => {
            let merged = config::load(explicit)?;
            if mode.is_structured() {
                format::print_structured(&merged, mode)
            } else {
                format::print_text(&toml::to_string_pretty(&merged)?)
            }
        }
        ConfigCmd::Paths => cmd_paths(explicit, mode),
        ConfigCmd::Validate => match config::load(explicit) {
            Ok(_) if mode.is_structured() => {
                format::print_structured(&serde_json::json!({"valid": true, "errors": []}), mode)
            }
            Ok(_) => format::print_text("Config is valid."),
            Err(e) if mode.is_structured() => format::print_structured(
                &serde_json::json!({"valid": false, "errors": [e.to_string()]}),
                mode,
            ),
            Err(e) => Err(anyhow::anyhow!("config validation failed: {e}")),
        },
    }
}

/// `rekindle config paths` — every layer, lowest precedence first.
fn cmd_paths(explicit: Option<&Path>, mode: OutputMode) -> anyhow::Result<()> {
    let layers = config::layers(explicit)?;
    let described: Vec<String> = layers
        .iter()
        .map(|layer| match layer {
            Layer::File(p) => p.display().to_string(),
            Layer::DropIns(p) => format!("{}/*.toml", p.display()),
            Layer::Required(p) => format!("{} (--config)", p.display()),
        })
        .collect();
    if mode.is_structured() {
        return format::print_list(&described, mode);
    }
    format::print_text("Config layers (lowest → highest precedence):")?;
    for (i, (layer, text)) in layers.iter().zip(&described).enumerate() {
        let exists = if layer.path().exists() {
            " (exists)"
        } else {
            ""
        };
        format::print_text(&format!("  {}. {text}{exists}", i + 1))?;
    }
    Ok(())
}
