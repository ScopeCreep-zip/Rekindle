//! Output formatting dispatch for CLI commands.
//!
//! Single formatting layer — command modules never call `println!` directly.
//! Quiet mode suppresses informational text but never structured output.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use super::OutputMode;

static QUIET: AtomicBool = AtomicBool::new(false);

/// Enable quiet mode. Called once from main.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Print as pretty-printed JSON.
pub fn print_json<T: Serialize + ?Sized>(value: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{json}")?;
    Ok(())
}

/// Print as single-line JSON (JSONL).
pub fn print_jsonl<T: Serialize + ?Sized>(value: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string(value)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{json}")?;
    Ok(())
}

/// Print a plain text line. Suppressed by --quiet.
pub fn print_text(msg: &str) -> anyhow::Result<()> {
    if is_quiet() {
        return Ok(());
    }
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{msg}")?;
    Ok(())
}

/// Print a value in the appropriate format for the current mode.
pub fn print_structured<T: Serialize + ?Sized>(value: &T, mode: OutputMode) -> anyhow::Result<()> {
    match mode {
        OutputMode::Jsonl => print_jsonl(value),
        _ => print_json(value),
    }
}

/// Print key-value pairs.
pub fn print_kv(pairs: &[(&str, String)], mode: OutputMode) -> anyhow::Result<()> {
    if mode.is_structured() {
        let obj: serde_json::Map<String, serde_json::Value> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), serde_json::Value::String(v.clone())))
            .collect();
        return print_structured(&obj, mode);
    }
    let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut stdout = std::io::stdout().lock();
    for (key, value) in pairs {
        writeln!(stdout, "  {key:<max_key_len$}  {value}")?;
    }
    Ok(())
}

/// Print a list of items.
pub fn print_list(items: &[String], mode: OutputMode) -> anyhow::Result<()> {
    if mode.is_structured() {
        return print_structured(items, mode);
    }
    let mut stdout = std::io::stdout().lock();
    for item in items {
        writeln!(stdout, "  {item}")?;
    }
    Ok(())
}

/// Print doctor check results.
pub fn print_doctor_checks(
    checks: &[rekindle_types::display::Check],
    mode: OutputMode,
    quiet: bool,
) -> anyhow::Result<()> {
    if mode.is_structured() {
        let output = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "version": env!("CARGO_PKG_VERSION"),
            "summary": {
                "pass": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Pass).count(),
                "warn": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Warn).count(),
                "fail": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Fail).count(),
            },
            "checks": checks.iter().map(|c| serde_json::json!({
                "id": c.id,
                "category": c.category,
                "status": match c.status {
                    rekindle_types::display::CheckStatus::Pass => "pass",
                    rekindle_types::display::CheckStatus::Warn => "warn",
                    rekindle_types::display::CheckStatus::Fail => "fail",
                },
                "value": c.value,
                "description": c.description,
            })).collect::<Vec<_>>(),
        });
        return print_structured(&output, mode);
    }

    if quiet { return Ok(()); }

    let mut stdout = std::io::stdout().lock();
    let mut current_category = "";

    for check in checks {
        if check.category != current_category {
            current_category = &check.category;
            writeln!(stdout, "\n{}", current_category.to_uppercase())?;
        }

        let icon = match check.status {
            rekindle_types::display::CheckStatus::Pass => "  [PASS]",
            rekindle_types::display::CheckStatus::Warn => "  [WARN]",
            rekindle_types::display::CheckStatus::Fail => "  [FAIL]",
        };

        writeln!(stdout, "  {icon} {:<35} {}", check.id, check.value)?;

        if check.status != rekindle_types::display::CheckStatus::Pass && !check.description.is_empty() {
            writeln!(stdout, "    {}", check.description)?;
        }
    }

    let pass = checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Pass).count();
    let warn = checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Warn).count();
    let fail = checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Fail).count();
    writeln!(stdout, "\n{pass} passed, {warn} warnings, {fail} failures")?;

    Ok(())
}

/// Print a multi-step ceremony header: "[N/M] Label"
pub fn step_header(step: u32, total: u32, label: &str) -> anyhow::Result<()> {
    if is_quiet() { return Ok(()); }
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "\n  [{step}/{total}] {label}")?;
    Ok(())
}

/// Print a step completion message.
pub fn step_done(msg: &str) -> anyhow::Result<()> {
    if is_quiet() { return Ok(()); }
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "        {msg} ... done")?;
    Ok(())
}

/// Print a step skip message.
pub fn step_skip(msg: &str) -> anyhow::Result<()> {
    if is_quiet() { return Ok(()); }
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "        {msg} ... (already done)")?;
    Ok(())
}
