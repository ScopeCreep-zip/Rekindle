//! Shared utility functions used across CLI command modules.
//!
//! Every function here is a boundary-layer primitive: input validation,
//! output formatting, TTY detection, tracing initialization, and secret
//! handling. No business logic — that lives in command modules.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;

/// Initialize tracing to a rolling daily log file.
///
/// Logs go to `${XDG_STATE_HOME}/rekindle/logs/rekindle.log`.
/// Never writes to stdout (would corrupt TUI alternate screen buffer).
/// Returns a guard that must be held for the lifetime of the program.
pub fn init_tracing() -> tracing_appender::non_blocking::WorkerGuard {
    let log_dir = dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("home directory must exist")
                .join(".local/state")
        })
        .join("rekindle/logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let file_appender = tracing_appender::rolling::daily(log_dir, "rekindle.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rekindle=info,warn".parse().expect("valid filter")),
        )
        .with_ansi(false)
        .init();

    guard
}

/// Path to the session state file.
///
/// `${XDG_STATE_HOME}/rekindle/session.json`
pub fn session_path() -> anyhow::Result<PathBuf> {
    let state_dir = dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("home directory must exist")
                .join(".local/state")
        })
        .join("rekindle");
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("failed to create state directory: {}", state_dir.display()))?;
    Ok(state_dir.join("session.json"))
}

/// Path to the config directory.
///
/// `${XDG_CONFIG_HOME}/rekindle/`
pub fn config_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?
        .join("rekindle");
    Ok(dir)
}

/// Path to the Veilid storage directory.
///
/// `${XDG_DATA_HOME}/rekindle/veilid/`
pub fn storage_dir(override_path: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }
    let dir = dirs::data_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("home directory must exist")
                .join(".local/share")
        })
        .join("rekindle/veilid");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create storage directory: {}", dir.display()))?;
    Ok(dir)
}

// ── Input Sanitization ──────────────────────────────────────────────────

/// Strip terminal escape sequences and control characters from
/// peer-controlled text before rendering.
///
/// Re-exported from `rekindle_node::validation`, which owns the single
/// implementation — this crate carried a byte-identical copy (differing
/// only in comments). The tests in `helpers/tests.rs` exercise it
/// through this path.
pub use rekindle_node::validation::sanitize_for_display;

/// Validate a display name.
///
/// Rules:
/// - 1-64 characters after trimming whitespace
/// - No control characters
/// - No leading/trailing whitespace (trimmed automatically)
pub fn validate_display_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("display name cannot be empty");
    }
    if trimmed.len() > 64 {
        anyhow::bail!("display name too long ({} chars, max 64)", trimmed.len());
    }
    if trimmed.chars().any(char::is_control) {
        anyhow::bail!("display name cannot contain control characters");
    }
    Ok(trimmed)
}

/// Validate a community or channel name.
///
/// Rules:
/// - 1-100 characters
/// - No control characters
/// - Alphanumeric, hyphens, underscores, spaces allowed
pub fn validate_name(name: &str, label: &str) -> anyhow::Result<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{label} name cannot be empty");
    }
    if trimmed.len() > 100 {
        anyhow::bail!("{label} name too long ({} chars, max 100)", trimmed.len());
    }
    if trimmed.chars().any(char::is_control) {
        anyhow::bail!("{label} name cannot contain control characters");
    }
    Ok(trimmed)
}

// ── TTY-Aware Prompts ───────────────────────────────────────────────────

/// Prompt for confirmation of a destructive operation.
///
/// In interactive mode: asks the user to type a confirmation phrase.
/// In non-interactive mode (piped stdin): returns an error.
///
/// Returns `true` if the user confirmed, `false` if they cancelled.
pub fn confirm_destructive(prompt: &str, phrase: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "destructive operation requires interactive confirmation\n\
             pass --yes to skip (if supported) or run in a terminal"
        );
    }

    let input: String = dialoguer::Input::new()
        .with_prompt(format!("{prompt}\nType \"{phrase}\" to confirm"))
        .interact_text()
        .context("failed to read confirmation")?;

    Ok(input.trim() == phrase)
}

/// Prompt for a yes/no confirmation.
///
/// Default is `false` (safe default — don't proceed if ambiguous).
pub fn confirm(prompt: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "confirmation required but stdin is not a terminal\n\
             pass --yes to skip or run in a terminal"
        );
    }

    dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .context("failed to read confirmation")
}

/// Prompt for a password with zeroize-on-drop.
///
/// In interactive mode: uses dialoguer's masked prompt.
/// In piped mode: reads from stdin, trims trailing newline.
/// Refuses empty passwords.
pub fn prompt_password(prompt: &str) -> anyhow::Result<zeroize::Zeroizing<String>> {
    if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .context("failed to read password from stdin")?;
        if buf.ends_with('\n') {
            buf.pop();
        }
        if buf.ends_with('\r') {
            buf.pop();
        }
        if buf.is_empty() {
            anyhow::bail!("empty password from stdin — refusing to proceed");
        }
        return Ok(zeroize::Zeroizing::new(buf));
    }

    let pass = dialoguer::Password::new()
        .with_prompt(prompt)
        .interact()
        .context("failed to read password")?;

    if pass.is_empty() {
        anyhow::bail!("empty password — refusing to proceed");
    }

    Ok(zeroize::Zeroizing::new(pass))
}

/// Prompt for a display name interactively, or use the provided value.
///
/// If `provided` is `Some`, validates and returns it.
/// If `None` and stdin is a TTY, prompts the user.
/// If `None` and stdin is piped, returns an error.
pub fn resolve_display_name(provided: Option<&str>) -> anyhow::Result<String> {
    if let Some(name) = provided {
        return validate_display_name(name);
    }

    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "display name required in non-interactive mode\n\
             pass --display-name <NAME>"
        );
    }

    let name: String = dialoguer::Input::new()
        .with_prompt("Display name")
        .interact_text()
        .context("failed to read display name")?;

    validate_display_name(&name)
}

// ── Formatting ──────────────────────────────────────────────────────────

/// Format a duration as a human-readable string.
///
/// Examples: "just now", "4m ago", "2h 13m ago", "3d 5h ago"
pub fn format_duration_ago(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    if hours < 24 {
        if rem_mins > 0 {
            return format!("{hours}h {rem_mins}m ago");
        }
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    let rem_hours = hours % 24;
    if rem_hours > 0 {
        format!("{days}d {rem_hours}h ago")
    } else {
        format!("{days}d ago")
    }
}

/// Format an epoch timestamp (milliseconds) as a human-readable local time.
pub fn format_timestamp(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let dt = Local.timestamp_millis_opt(epoch_ms.cast_signed());
    match dt.single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{epoch_ms}ms"),
    }
}

/// Format an epoch timestamp as a short time (HH:MM).
pub fn format_time_short(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let dt = Local.timestamp_millis_opt(epoch_ms.cast_signed());
    match dt.single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => "??:??".to_string(),
    }
}

/// Format bytes as a human-readable string.
///
/// Examples: "42 B", "1.2 KB", "3.4 MB", "1.1 GB"
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Select the unit via integer division, then render one decimal place
    // using fixed-point integer math so no precision-losing float cast is
    // needed (tenths = value * 10 / divisor).
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    let (divisor, unit) = if bytes < MB {
        (KB, "KB")
    } else if bytes < GB {
        (MB, "MB")
    } else {
        (GB, "GB")
    };
    let whole = bytes / divisor;
    let tenths = (bytes % divisor) * 10 / divisor;
    format!("{whole}.{tenths} {unit}")
}

/// Abbreviate a hex key for display: first 8 chars + "..." + last 4.
pub fn abbreviate_key(key: &str) -> String {
    if key.len() > 16 {
        format!("{}...{}", &key[..8], &key[key.len() - 4..])
    } else {
        key.to_string()
    }
}

/// Format an uptime duration as a compact string.
///
/// Examples: "42s", "12m 34s", "5h 12m", "3d 5h"
pub fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        let rem = secs % 60;
        return format!("{mins}m {rem}s");
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    if hours < 24 {
        return format!("{hours}h {rem_mins}m");
    }
    let days = hours / 24;
    let rem_hours = hours % 24;
    format!("{days}d {rem_hours}h")
}

/// Resolve a channel name or ID within a community.
///
/// This is a placeholder that will be implemented when channel resolution
/// is wired through the query engine. For now, returns the input as-is
/// (callers use it as a channel ID).
pub fn resolve_channel_id(channel: &str) -> String {
    channel.to_string()
}

// ── Audit log ───────────────────────────────────────────────────────────

/// Append a destructive action to the audit log.
///
/// The audit log at `${XDG_STATE_HOME}/rekindle/audit.jsonl` records
/// every destructive operation (leave community, remove friend, block
/// peer, rotate MEK, destroy identity) with a BLAKE3 hash chain for
/// tamper detection.
///
/// Each entry is a single JSON line:
/// ```json
/// {"ts":"2026-04-30T23:00:00Z","action":"leave_community","target":"dev-team","outcome":"ok","prev_hash":"..."}
/// ```
///
/// This is best-effort — audit failures are logged but don't block the
/// operation. An inaccessible log directory or full disk should not
/// prevent the user from leaving a community.
pub fn audit_log(action: &str, target: &str, outcome: &str) {
    let result = audit_log_inner(action, target, outcome);
    if let Err(e) = result {
        tracing::warn!(error = %e, action, target, "audit log write failed (non-fatal)");
    }
}

fn audit_log_inner(action: &str, target: &str, outcome: &str) -> anyhow::Result<()> {
    use std::io::Write;

    let log_dir = dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state")
        })
        .join("rekindle");
    std::fs::create_dir_all(&log_dir)?;

    let path = log_dir.join("audit.jsonl");

    // Read last line to compute hash chain
    let prev_hash = match std::fs::read_to_string(&path) {
        Ok(content) => content.lines().last().map_or_else(
            || "genesis".to_string(),
            |line| {
                let hash = blake3::hash(line.as_bytes());
                hex::encode(&hash.as_bytes()[..16])
            },
        ),
        Err(_) => "genesis".to_string(),
    };

    let ts = chrono::Utc::now().to_rfc3339();
    let entry = serde_json::json!({
        "ts": ts,
        "action": action,
        "target": sanitize_for_display(target),
        "outcome": outcome,
        "prev_hash": prev_hash,
    });

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;

    Ok(())
}

#[cfg(test)]
#[path = "helpers/tests.rs"]
mod tests;
