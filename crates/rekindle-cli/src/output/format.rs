//! Output formatting dispatch for CLI commands.
//!
//! Every command calls these functions to produce output in the correct
//! format (text, JSON, JSONL). This is the single formatting layer —
//! command modules never call `println!` directly.
//!
//! Functions in this module are allowed to write to stdout via the
//! `writeln!` macro on a `std::io::Stdout` handle.

use std::io::Write;

use serde::Serialize;

use super::OutputMode;

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
pub fn print_text(msg: &str) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{msg}")?;
    Ok(())
}

/// Print a value as JSON or JSONL depending on mode.
pub fn print_structured<T: Serialize + ?Sized>(value: &T, mode: OutputMode) -> anyhow::Result<()> {
    match mode {
        OutputMode::Json => print_json(value),
        OutputMode::Jsonl => print_jsonl(value),
        _ => anyhow::bail!("print_structured called in non-structured mode"),
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
    checks: &[crate::doctor::Check],
    mode: OutputMode,
    quiet: bool,
) -> anyhow::Result<()> {
    if mode.is_structured() {
        let output = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "version": env!("CARGO_PKG_VERSION"),
            "summary": {
                "pass": checks.iter().filter(|c| c.status == crate::doctor::Status::Pass).count(),
                "warn": checks.iter().filter(|c| c.status == crate::doctor::Status::Warn).count(),
                "fail": checks.iter().filter(|c| c.status == crate::doctor::Status::Fail).count(),
            },
            "checks": checks.iter().map(|c| serde_json::json!({
                "id": c.id,
                "category": c.category,
                "status": match c.status {
                    crate::doctor::Status::Pass => "pass",
                    crate::doctor::Status::Warn => "warn",
                    crate::doctor::Status::Fail => "fail",
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
            current_category = check.category;
            writeln!(stdout, "\n{}", current_category.to_uppercase())?;
        }

        let icon = match check.status {
            crate::doctor::Status::Pass => "  [PASS]",
            crate::doctor::Status::Warn => "  [WARN]",
            crate::doctor::Status::Fail => "  [FAIL]",
        };

        writeln!(stdout, "  {icon} {:<35} {}", check.id, check.value)?;

        if check.status != crate::doctor::Status::Pass && !check.description.is_empty() {
            writeln!(stdout, "    {}", check.description)?;
        }
    }

    // Summary line
    let pass = checks
        .iter()
        .filter(|c| c.status == crate::doctor::Status::Pass)
        .count();
    let warn = checks
        .iter()
        .filter(|c| c.status == crate::doctor::Status::Warn)
        .count();
    let fail = checks
        .iter()
        .filter(|c| c.status == crate::doctor::Status::Fail)
        .count();
    writeln!(stdout, "\n{pass} passed, {warn} warnings, {fail} failures")?;

    Ok(())
}

/// Print a step header for multi-step ceremonies (init, join, etc.).
///
/// Format: `[N/M] Label`
pub fn step_header(step: u32, total: u32, label: &str) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "\n  [{step}/{total}] {label}")?;
    Ok(())
}

/// Print a step completion message.
pub fn step_done(msg: &str) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "        {msg} ... done")?;
    Ok(())
}

/// Print a step skip message.
pub fn step_skip(msg: &str) -> anyhow::Result<()> {
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
        let checks = vec![
            crate::doctor::Check {
                id: "node.running".into(),
                category: "node",
                status: crate::doctor::Status::Pass,
                value: "active".into(),
                description: String::new(),
            },
            crate::doctor::Check {
                id: "crypto.prekeys.low".into(),
                category: "crypto",
                status: crate::doctor::Status::Warn,
                value: "3 remaining".into(),
                description: "replenish prekeys".into(),
            },
        ];

        // Verify the JSON structure matches the contract
        let output = serde_json::json!({
            "summary": {
                "pass": checks.iter().filter(|c| c.status == crate::doctor::Status::Pass).count(),
                "warn": checks.iter().filter(|c| c.status == crate::doctor::Status::Warn).count(),
                "fail": checks.iter().filter(|c| c.status == crate::doctor::Status::Fail).count(),
            },
        });
        assert_eq!(output["summary"]["pass"], 1);
        assert_eq!(output["summary"]["warn"], 1);
        assert_eq!(output["summary"]["fail"], 0);
    }
}
