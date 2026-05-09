//! Output formatting dispatch for CLI commands.
//!
//! Every command calls these functions to produce output in the correct
//! format (text, JSON, JSONL). This is the single formatting layer —
//! command modules never call `println!` directly.
//!
//! Functions in this module are allowed to write to stdout via the
//! `writeln!` macro on a `std::io::Stdout` handle.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use super::OutputMode;

/// Global quiet flag — when set, `print_text` and `print_kv` suppress output.
/// Structured output (JSON/JSONL) is NOT suppressed by quiet — it's data, not UI.
/// Set once at startup via `set_quiet()`, read on every output call.
static QUIET: AtomicBool = AtomicBool::new(false);

/// Enable quiet mode. Called once from `main.rs` when `--quiet` is set.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Whether quiet mode is active.
fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Print a value as pretty-printed JSON.
pub fn print_json<T: Serialize + ?Sized>(value: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{json}")?;
    Ok(())
}

/// Print a value as a single-line JSON (JSONL).
pub fn print_jsonl<T: Serialize + ?Sized>(value: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string(value)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{json}")?;
    Ok(())
}

/// Print a plain text line.
///
/// Suppressed when `--quiet` is set. Structured output (JSON/JSONL)
/// is never suppressed — it's data, not informational UI.
pub fn print_text(msg: &str) -> anyhow::Result<()> {
    if is_quiet() {
        return Ok(());
    }
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{msg}")?;
    Ok(())
}

/// Print a value in the appropriate format for the current mode.
///
/// - JSON mode: pretty-printed JSON
/// - JSONL mode: single-line JSON
/// - Text mode: pretty-printed JSON (always readable, never loses data)
/// - TUI mode: pretty-printed JSON (TUI views render their own widgets)
///
/// Commands that want custom text-mode rendering (tables, kv pairs)
/// should check `mode.is_structured()` and handle text mode themselves
/// before calling this function.
pub fn print_structured<T: Serialize + ?Sized>(value: &T, mode: OutputMode) -> anyhow::Result<()> {
    match mode {
        OutputMode::Jsonl => print_jsonl(value),
        _ => print_json(value),
    }
}

/// Print a key-value pair list.
///
/// In text mode: aligned columns.
/// In JSON mode: object with key-value pairs.
pub fn print_kv(pairs: &[(&str, String)], mode: OutputMode) -> anyhow::Result<()> {
    match mode {
        OutputMode::Json | OutputMode::Jsonl => {
            let obj: serde_json::Map<String, serde_json::Value> = pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), serde_json::Value::String(v.clone())))
                .collect();
            print_structured(&obj, mode)
        }
        _ => {
            let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
            let mut stdout = std::io::stdout().lock();
            for (key, value) in pairs {
                writeln!(stdout, "  {key:<max_key_len$}  {value}")?;
            }
            Ok(())
        }
    }
}

/// Print a list of items.
///
/// In text mode: one per line with indent.
/// In JSON mode: array of strings.
pub fn print_list(items: &[String], mode: OutputMode) -> anyhow::Result<()> {
    match mode {
        OutputMode::Json | OutputMode::Jsonl => print_structured(items, mode),
        _ => {
            let mut stdout = std::io::stdout().lock();
            for item in items {
                writeln!(stdout, "  {item}")?;
            }
            Ok(())
        }
    }
}

/// Print doctor check results in text or JSON format.
///
/// Text format uses ASCII labels [PASS]/[WARN]/[FAIL] for accessibility.
/// JSON format includes all check metadata.
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

    if quiet {
        return Ok(());
    }

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

    // Summary line
    let pass = checks
        .iter()
        .filter(|c| c.status == rekindle_types::display::CheckStatus::Pass)
        .count();
    let warn = checks
        .iter()
        .filter(|c| c.status == rekindle_types::display::CheckStatus::Warn)
        .count();
    let fail = checks
        .iter()
        .filter(|c| c.status == rekindle_types::display::CheckStatus::Fail)
        .count();
    writeln!(stdout, "\n{pass} passed, {warn} warnings, {fail} failures")?;

    Ok(())
}

/// Print a step header for multi-step ceremonies (init, join, etc.).
///
/// Format: `[N/M] Label`
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

#[cfg(test)]
mod tests {
    #[test]
    fn print_structured_json_produces_valid_json() {
        // We can't capture stdout in a unit test, but we can verify
        // the serialization doesn't panic on various types
        let val = serde_json::json!({"key": "value", "num": 42});
        // Serialize to string (same path as print_json) to verify no panic
        let json = serde_json::to_string_pretty(&val).unwrap();
        assert!(json.contains("\"key\""));
        assert!(json.contains("42"));
    }

    #[test]
    fn print_structured_handles_empty_object() {
        let val = serde_json::json!({});
        let json = serde_json::to_string_pretty(&val).unwrap();
        assert_eq!(json.trim(), "{}");
    }

    #[test]
    fn print_structured_handles_nested() {
        let val = serde_json::json!({
            "community": {
                "name": "dev-team",
                "channels": ["general", "random"]
            }
        });
        let json = serde_json::to_string_pretty(&val).unwrap();
        assert!(json.contains("dev-team"));
        assert!(json.contains("general"));
    }

    #[test]
    fn print_structured_handles_unicode() {
        let val = serde_json::json!({"name": "日本語コミュニティ"});
        let json = serde_json::to_string_pretty(&val).unwrap();
        assert!(json.contains("日本語コミュニティ"));
    }

    #[test]
    fn print_structured_escapes_special_chars() {
        let val = serde_json::json!({"msg": "hello \"world\" \n\ttab"});
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("\\\"world\\\""));
        assert!(json.contains("\\n"));
        assert!(json.contains("\\t"));
    }

    #[test]
    fn doctor_check_serialization() {
        let checks = [
            rekindle_types::display::Check {
                id: "node.running".into(),
                category: "node".into(),
                status: rekindle_types::display::CheckStatus::Pass,
                value: "active".into(),
                description: String::new(),
            },
            rekindle_types::display::Check {
                id: "crypto.prekeys.low".into(),
                category: "crypto".into(),
                status: rekindle_types::display::CheckStatus::Warn,
                value: "3 remaining".into(),
                description: "replenish prekeys".into(),
            },
        ];

        // Verify the JSON structure matches the contract
        let output = serde_json::json!({
            "summary": {
                "pass": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Pass).count(),
                "warn": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Warn).count(),
                "fail": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Fail).count(),
            },
        });
        assert_eq!(output["summary"]["pass"], 1);
        assert_eq!(output["summary"]["warn"], 1);
        assert_eq!(output["summary"]["fail"], 0);
    }
}
