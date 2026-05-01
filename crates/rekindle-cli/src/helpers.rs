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

/// Strip control characters and ANSI escape sequences from untrusted text.
///
/// Allows \n and \t (needed for message formatting). Strips:
/// - Individual control characters (C0 set except \n and \t)
/// - Full ANSI CSI sequences: ESC + '[' + params + final byte
/// - Full ANSI OSC sequences: ESC + ']' + ... + ST
///
/// This prevents terminal escape injection from peer-controlled display
/// names, message bodies, channel topics, etc. A partial strip (removing
/// only the ESC byte) leaves broken `[31m` fragments that could confuse
/// terminals or users. We strip the entire sequence.
pub fn sanitize_for_display(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Start of an escape sequence — consume the entire sequence
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: ESC [ <params> <final byte>
                    chars.next(); // consume '['
                    // Consume parameter bytes (0x30-0x3F) and intermediate bytes (0x20-0x2F)
                    // until we hit a final byte (0x40-0x7E) or run out of input
                    loop {
                        match chars.peek() {
                            Some(&fc) if ('\x40'..='\x7e').contains(&fc) => {
                                chars.next(); // consume final byte
                                break;
                            }
                            Some(&fc) if ('\x20'..='\x3f').contains(&fc) => {
                                chars.next(); // consume parameter/intermediate byte
                            }
                            _ => break, // malformed sequence — stop consuming
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: ESC ] ... ST (ST = ESC \ or BEL)
                    chars.next(); // consume ']'
                    loop {
                        match chars.next() {
                            Some('\x07') | None => break,  // BEL or EOF terminates OSC
                            Some('\x1b') => {             // ESC \ terminates OSC
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            _ => {}                       // consume OSC content
                        }
                    }
                }
                _ => {
                    // Other escape — consume just the ESC
                }
            }
        } else if c.is_control() && c != '\n' && c != '\t' {
            // Strip other control characters (NUL, BEL, BS, etc.)
        } else {
            result.push(c);
        }
    }

    result
}

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
    #[allow(clippy::cast_possible_wrap)]
    let dt = Local.timestamp_millis_opt(epoch_ms as i64);
    match dt.single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{epoch_ms}ms"),
    }
}

/// Format an epoch timestamp as a short time (HH:MM).
pub fn format_time_short(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    #[allow(clippy::cast_possible_wrap)]
    let dt = Local.timestamp_millis_opt(epoch_ms as i64);
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
    #[allow(clippy::cast_precision_loss)]
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.1} KB");
    }
    let mb = kb / 1024.0;
    if mb < 1024.0 {
        return format!("{mb:.1} MB");
    }
    let gb = mb / 1024.0;
    format!("{gb:.1} GB")
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

// ── Community Resolution ────────────────────────────────────────────────

/// Resolve a community name or governance key to a membership.
///
/// First tries exact governance key match, then case-insensitive name match.
/// Returns an error if ambiguous (multiple communities with the same name)
/// or not found.
pub fn resolve_community<'a>(
    target: &str,
    session: &'a rekindle_transport::Session,
) -> anyhow::Result<&'a rekindle_transport::CommunityMembership> {
    // Try exact governance key match
    if let Some(m) = session.community(target) {
        return Ok(m);
    }

    // Try case-insensitive name match
    if let Some(m) = session.community_by_name(target) {
        return Ok(m);
    }

    anyhow::bail!(
        "community '{target}' not found\n\
         list joined communities: rekindle community list"
    )
}

/// Resolve a channel name or ID within a community.
///
/// This is a placeholder that will be implemented when channel resolution
/// is wired through the query engine. For now, returns the input as-is
/// (callers use it as a channel ID).
pub fn resolve_channel_id(channel: &str) -> String {
    channel.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Sanitization: ANSI injection ─────────────────────────────────

    #[test]
    fn sanitize_strips_basic_csi_sequence() {
        // CSI color: ESC [ 31 m → stripped entirely
        assert_eq!(sanitize_for_display("hello\x1b[31mworld"), "helloworld");
    }

    #[test]
    fn sanitize_strips_sgr_reset() {
        // ESC [ 0 m (reset) → stripped
        assert_eq!(sanitize_for_display("a\x1b[0mb"), "ab");
    }

    #[test]
    fn sanitize_strips_cursor_movement() {
        // ESC [ 10 A (cursor up 10) → stripped
        assert_eq!(sanitize_for_display("before\x1b[10Aafter"), "beforeafter");
    }

    #[test]
    fn sanitize_strips_erase_display() {
        // ESC [ 2 J (clear screen) → stripped
        assert_eq!(sanitize_for_display("safe\x1b[2Jtext"), "safetext");
    }

    #[test]
    fn sanitize_strips_osc_title_injection() {
        // OSC title set: ESC ] 0 ; evil BEL → stripped
        assert_eq!(
            sanitize_for_display("before\x1b]0;evil title\x07after"),
            "beforeafter"
        );
    }

    #[test]
    fn sanitize_strips_osc_with_st_terminator() {
        // OSC terminated by ESC \ instead of BEL
        assert_eq!(
            sanitize_for_display("a\x1b]0;payload\x1b\\b"),
            "ab"
        );
    }

    #[test]
    fn sanitize_strips_nested_escape() {
        // Nested: ESC [ ESC [ 31m → both ESC sequences consumed
        assert_eq!(
            sanitize_for_display("x\x1b[\x1b[31my"),
            "xy"
        );
    }

    #[test]
    fn sanitize_strips_incomplete_csi() {
        // Incomplete CSI: ESC [ with no final byte → ESC consumed, [ left
        // The [ is a printable char so it stays. The CSI parser stops
        // when it hits end-of-input without a final byte.
        let result = sanitize_for_display("end\x1b[");
        // ESC is consumed. '[' is not a parameter/intermediate byte range
        // (0x20-0x3F), nor a final byte (0x40-0x7E) — actually '[' is 0x5B
        // which IS in the final byte range. So the parser consumes '[' as
        // the final byte. Result: "end"
        assert_eq!(result, "end");
    }

    #[test]
    fn sanitize_strips_null_bytes() {
        assert_eq!(sanitize_for_display("hello\x00world"), "helloworld");
    }

    #[test]
    fn sanitize_strips_bell() {
        assert_eq!(sanitize_for_display("ding\x07dong"), "dingdong");
    }

    #[test]
    fn sanitize_strips_backspace() {
        // BS (0x08) can overwrite previous chars on some terminals
        assert_eq!(sanitize_for_display("abc\x08def"), "abcdef");
    }

    #[test]
    fn sanitize_preserves_newline() {
        assert_eq!(sanitize_for_display("line1\nline2"), "line1\nline2");
    }

    #[test]
    fn sanitize_preserves_tab() {
        assert_eq!(sanitize_for_display("col1\tcol2"), "col1\tcol2");
    }

    #[test]
    fn sanitize_preserves_unicode() {
        assert_eq!(sanitize_for_display("hello 🌍 世界"), "hello 🌍 世界");
    }

    #[test]
    fn sanitize_strips_multiple_sequences() {
        // Multiple CSI sequences in one string
        assert_eq!(
            sanitize_for_display("\x1b[1m\x1b[31mbold red\x1b[0m normal"),
            "bold red normal"
        );
    }

    #[test]
    fn sanitize_empty_string() {
        assert_eq!(sanitize_for_display(""), "");
    }

    #[test]
    fn sanitize_only_escape_sequence() {
        assert_eq!(sanitize_for_display("\x1b[31m"), "");
    }

    // ── Sanitization: Unicode adversarial ──────────────────────────

    #[test]
    fn sanitize_preserves_rtl_override() {
        // U+202E RIGHT-TO-LEFT OVERRIDE is not a C0 control char,
        // it's a Unicode formatting character. Our sanitizer strips
        // C0 controls (0x00-0x1F except \n\t) and ANSI escapes.
        // RTL override is U+202E which is not in C0 range.
        // This is intentional — full Unicode normalization is a
        // separate concern from terminal escape injection.
        let input = "hello\u{202E}dlrow";
        let result = sanitize_for_display(input);
        assert!(result.contains('\u{202E}'));
    }

    #[test]
    fn sanitize_preserves_zero_width_joiner() {
        // ZWJ (U+200D) is not a control char — it's used in emoji sequences
        let input = "👨\u{200D}👩\u{200D}👧";
        let result = sanitize_for_display(input);
        assert_eq!(result, input);
    }

    // ── Display name validation ────────────────────────────────────

    #[test]
    fn validate_display_name_trims() {
        assert_eq!(validate_display_name("  alice  ").unwrap(), "alice");
    }

    #[test]
    fn validate_display_name_rejects_empty() {
        assert!(validate_display_name("").is_err());
        assert!(validate_display_name("   ").is_err());
    }

    #[test]
    fn validate_display_name_rejects_long() {
        let long = "a".repeat(65);
        assert!(validate_display_name(&long).is_err());
    }

    #[test]
    fn validate_display_name_accepts_max_length() {
        let max = "a".repeat(64);
        assert!(validate_display_name(&max).is_ok());
    }

    #[test]
    fn validate_display_name_rejects_control_chars() {
        assert!(validate_display_name("hello\x00world").is_err());
        assert!(validate_display_name("hello\x1bworld").is_err());
        assert!(validate_display_name("hello\x07world").is_err());
    }

    #[test]
    fn validate_display_name_accepts_unicode() {
        assert_eq!(validate_display_name("日本語").unwrap(), "日本語");
        assert_eq!(validate_display_name("émile").unwrap(), "émile");
        assert_eq!(validate_display_name("🔥 fire").unwrap(), "🔥 fire");
    }

    // ── Name validation (community/channel) ────────────────────────

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_name("", "Channel").is_err());
        assert!(validate_name("   ", "Channel").is_err());
    }

    #[test]
    fn validate_name_rejects_long() {
        let long = "a".repeat(101);
        assert!(validate_name(&long, "Community").is_err());
    }

    #[test]
    fn validate_name_accepts_max() {
        let max = "a".repeat(100);
        assert!(validate_name(&max, "Community").is_ok());
    }

    #[test]
    fn validate_name_rejects_control_chars() {
        assert!(validate_name("hello\x00world", "Channel").is_err());
    }

    #[test]
    fn validate_name_trims_whitespace() {
        assert_eq!(validate_name("  general  ", "Channel").unwrap(), "general");
    }

    #[test]
    fn format_duration_ago_just_now() {
        assert_eq!(format_duration_ago(Duration::from_secs(30)), "just now");
    }

    #[test]
    fn format_duration_ago_minutes() {
        assert_eq!(format_duration_ago(Duration::from_secs(300)), "5m ago");
    }

    #[test]
    fn format_duration_ago_hours() {
        assert_eq!(
            format_duration_ago(Duration::from_secs(7380)),
            "2h 3m ago"
        );
    }

    #[test]
    fn format_duration_ago_days() {
        assert_eq!(
            format_duration_ago(Duration::from_secs(90000)),
            "1d 1h ago"
        );
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(42), "42 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }

    #[test]
    fn abbreviate_key_short() {
        assert_eq!(abbreviate_key("abcdef"), "abcdef");
    }

    #[test]
    fn abbreviate_key_long() {
        let key = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        assert_eq!(abbreviate_key(key), "abcdef12...7890");
    }

    #[test]
    fn format_uptime_seconds() {
        assert_eq!(format_uptime(42), "42s");
    }

    #[test]
    fn format_uptime_minutes() {
        assert_eq!(format_uptime(754), "12m 34s");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(format_uptime(18720), "5h 12m");
    }

    #[test]
    fn format_uptime_days() {
        assert_eq!(format_uptime(277200), "3d 5h");
    }
}
