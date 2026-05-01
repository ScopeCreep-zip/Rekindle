//! Table formatting using comfy-table with UTF-8 and ASCII presets.
//!
//! Wraps comfy-table to provide mode-aware table output. In structured
//! modes (JSON/JSONL), tables are serialized as arrays of objects.
//! In text mode, tables use UTF-8 box-drawing borders by default,
//! falling back to ASCII if TERM=dumb.

use std::io::Write;

use super::OutputMode;
use super::color::ColorSupport;

/// Print a table with headers and rows.
///
/// In structured modes: serializes as an array of objects.
/// In text mode: renders a bordered table.
pub fn print_table(
    headers: &[&str],
    rows: &[Vec<String>],
    mode: OutputMode,
) -> anyhow::Result<()> {
    match mode {
        OutputMode::Json => {
            let objects: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    let map: serde_json::Map<String, serde_json::Value> = headers
                        .iter()
                        .zip(row.iter())
                        .map(|(h, v)| ((*h).to_string(), serde_json::Value::String(v.clone())))
                        .collect();
                    serde_json::Value::Object(map)
                })
                .collect();
            super::format::print_json(&objects)
        }
        OutputMode::Jsonl => {
            for row in rows {
                let obj: serde_json::Map<String, serde_json::Value> = headers
                    .iter()
                    .zip(row.iter())
                    .map(|(h, v)| ((*h).to_string(), serde_json::Value::String(v.clone())))
                    .collect();
                super::format::print_jsonl(&serde_json::Value::Object(obj))?;
            }
            Ok(())
        }
        _ => {
            let color_support = mode.color_support();
            let mut table = comfy_table::Table::new();

            // Use UTF-8 borders by default, ASCII if terminal doesn't support Unicode
            if ColorSupport::use_unicode() {
                table.load_preset(comfy_table::presets::UTF8_FULL);
            } else {
                table.load_preset(comfy_table::presets::ASCII_FULL);
            }

            // When piped or in non-TTY context, comfy-table disables styling.
            // force_no_tty + enforce_styling overrides that so tables render
            // consistently in CI and script pipelines that still want borders.
            // When color is available, enforce styling so ANSI output is active.
            if color_support.has_256_colors() || color_support.has_true_color() {
                table.enforce_styling();
            }
            if !color_support.is_enabled() {
                table.force_no_tty();
            }

            table.set_header(headers);
            for row in rows {
                table.add_row(row);
            }

            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{table}")?;
            Ok(())
        }
    }
}

/// Print a simple two-column table (key-value style).
///
/// Uses minimal borders — just column separation, no outer borders.
pub fn print_kv_table(
    pairs: &[(&str, String)],
    mode: OutputMode,
) -> anyhow::Result<()> {
    if mode.is_structured() {
        let obj: serde_json::Map<String, serde_json::Value> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), serde_json::Value::String(v.clone())))
            .collect();
        return super::format::print_structured(&serde_json::Value::Object(obj), mode);
    }

    let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut stdout = std::io::stdout().lock();
    for (key, value) in pairs {
        writeln!(stdout, "  {key:<max_key_len$}  {value}")?;
    }
    Ok(())
}
