//! Input validation for the daemon security boundary.
//!
//! Every string field crossing the IPC boundary is validated here before
//! reaching transport operations. This is the authoritative validation
//! layer — the CLI may also validate for UX, but the daemon enforces.
//!
//! Adapted from `rekindle-cli/src/helpers.rs` validation functions.
//! The daemon's versions return `IpcResponse::Error` directly so dispatch
//! handlers can `?`-propagate validation failures.

use crate::ipc::protocol::IpcResponse;

/// Maximum message body length (channel messages and DMs).
const MAX_MESSAGE_BODY: usize = 2000;

/// Maximum display name length.
const MAX_DISPLAY_NAME: usize = 64;

/// Maximum community/channel name length.
const MAX_NAME: usize = 100;

/// Validate a display name.
///
/// Rules:
/// - 1-64 characters after trimming
/// - No control characters (C0 set)
/// - Returns the trimmed string on success
pub fn validate_display_name(name: &str) -> Result<String, IpcResponse> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(IpcResponse::error(400, "display name cannot be empty"));
    }
    if trimmed.len() > MAX_DISPLAY_NAME {
        return Err(IpcResponse::error(
            400,
            format!(
                "display name too long ({} chars, max {MAX_DISPLAY_NAME})",
                trimmed.len()
            ),
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(IpcResponse::error(
            400,
            "display name cannot contain control characters",
        ));
    }
    Ok(trimmed.to_string())
}

/// Validate a community or channel name.
///
/// Rules:
/// - 1-100 characters after trimming
/// - No control characters
pub fn validate_name(name: &str, label: &str) -> Result<String, IpcResponse> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(IpcResponse::error(
            400,
            format!("{label} name cannot be empty"),
        ));
    }
    if trimmed.len() > MAX_NAME {
        return Err(IpcResponse::error(
            400,
            format!(
                "{label} name too long ({} chars, max {MAX_NAME})",
                trimmed.len()
            ),
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(IpcResponse::error(
            400,
            format!("{label} name cannot contain control characters"),
        ));
    }
    Ok(trimmed.to_string())
}

/// Validate a message body (channel message or DM).
///
/// Rules:
/// - 1-2000 characters
/// - Empty strings rejected
pub fn validate_message_body(body: &str) -> Result<(), IpcResponse> {
    if body.is_empty() {
        return Err(IpcResponse::error(400, "message body cannot be empty"));
    }
    if body.len() > MAX_MESSAGE_BODY {
        return Err(IpcResponse::error(
            400,
            format!(
                "message body too long ({} chars, max {MAX_MESSAGE_BODY})",
                body.len()
            ),
        ));
    }
    Ok(())
}

/// Validate a presence status string.
///
/// Must be one of: online, away, busy, invisible.
pub fn validate_status(status: &str) -> Result<(), IpcResponse> {
    match status {
        "online" | "away" | "busy" | "invisible" => Ok(()),
        other => Err(IpcResponse::error(
            400,
            format!("invalid status '{other}' — expected: online, away, busy, invisible"),
        )),
    }
}

/// Validate a key string (governance key, public key, invite code, etc.).
///
/// Rules:
/// - Non-empty
/// - No control characters
/// - Allowed characters: alphanumeric, Base64url (`-`, `_`), hex,
///   Veilid key separators (`:`), and padding (`+`, `/`, `=`)
///
/// Veilid keys use the format `VLD0:Base64url:Base64url` where the
/// Base64url segments contain `[A-Za-z0-9_-]`. Public keys are hex.
/// Invite codes may use other safe formats. This validation rejects
/// injection-dangerous characters while accepting all known key formats.
pub fn validate_key(key: &str, label: &str) -> Result<(), IpcResponse> {
    if key.is_empty() {
        return Err(IpcResponse::error(400, format!("{label} cannot be empty")));
    }
    if key.chars().any(char::is_control) {
        return Err(IpcResponse::error(
            400,
            format!("{label} cannot contain control characters"),
        ));
    }
    for ch in key.chars() {
        if !ch.is_ascii_alphanumeric() && !matches!(ch, ':' | '-' | '_' | '+' | '/' | '=' | '.') {
            return Err(IpcResponse::error(
                400,
                format!("{label} contains invalid character '{ch}'"),
            ));
        }
    }
    Ok(())
}

/// Validate a channel kind string.
pub fn validate_channel_kind(kind: &str) -> Result<(), IpcResponse> {
    match kind {
        "text" | "voice" | "announcement" | "forum" | "stage" | "directory" | "media"
        | "events" => Ok(()),
        other => Err(IpcResponse::error(
            400,
            format!(
                "invalid channel kind '{other}' — expected: text, voice, announcement, \
                 forum, stage, directory, media, events"
            ),
        )),
    }
}

/// Strip control characters and ANSI escape sequences from untrusted text.
///
/// Allows \n and \t (needed for message formatting). Strips:
/// - Individual control characters (C0 set except \n and \t)
/// - Full ANSI CSI sequences: ESC + '[' + params + final byte
/// - Full ANSI OSC sequences: ESC + ']' + ... + ST
pub fn sanitize_for_display(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    loop {
                        match chars.peek() {
                            Some(&fc) if ('\x40'..='\x7e').contains(&fc) => {
                                chars.next();
                                break;
                            }
                            Some(&fc) if ('\x20'..='\x3f').contains(&fc) => {
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    loop {
                        match chars.next() {
                            Some('\x07') | None => break,
                            Some('\x1b') => {
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        } else if c.is_control() && c != '\n' && c != '\t' {
            // Strip C0 controls except newline and tab
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Display name validation ────────────────────────────────────

    #[test]
    fn display_name_trims_whitespace() {
        assert_eq!(validate_display_name("  alice  ").unwrap(), "alice");
    }

    #[test]
    fn display_name_rejects_empty() {
        assert!(validate_display_name("").is_err());
        assert!(validate_display_name("   ").is_err());
    }

    #[test]
    fn display_name_rejects_too_long() {
        let long = "a".repeat(65);
        assert!(validate_display_name(&long).is_err());
    }

    #[test]
    fn display_name_accepts_max_length() {
        let max = "a".repeat(64);
        assert!(validate_display_name(&max).is_ok());
    }

    #[test]
    fn display_name_rejects_control_chars() {
        assert!(validate_display_name("hello\x00world").is_err());
        assert!(validate_display_name("hello\x1bworld").is_err());
    }

    #[test]
    fn display_name_accepts_unicode() {
        assert_eq!(validate_display_name("日本語").unwrap(), "日本語");
    }

    // ── Name validation ──────────────────────────���─────────────────

    #[test]
    fn name_rejects_empty() {
        assert!(validate_name("", "Channel").is_err());
    }

    #[test]
    fn name_rejects_too_long() {
        let long = "a".repeat(101);
        assert!(validate_name(&long, "Community").is_err());
    }

    #[test]
    fn name_accepts_max() {
        let max = "a".repeat(100);
        assert!(validate_name(&max, "Community").is_ok());
    }

    // ── Message body validation ────────────────────────────────────

    #[test]
    fn body_rejects_empty() {
        assert!(validate_message_body("").is_err());
    }

    #[test]
    fn body_rejects_too_long() {
        let long = "a".repeat(2001);
        assert!(validate_message_body(&long).is_err());
    }

    #[test]
    fn body_accepts_max() {
        let max = "a".repeat(2000);
        assert!(validate_message_body(&max).is_ok());
    }

    // ── Status validation ──────────────────────────────────────────

    #[test]
    fn status_accepts_valid() {
        assert!(validate_status("online").is_ok());
        assert!(validate_status("away").is_ok());
        assert!(validate_status("busy").is_ok());
        assert!(validate_status("invisible").is_ok());
    }

    #[test]
    fn status_rejects_invalid() {
        assert!(validate_status("unknown").is_err());
        assert!(validate_status("").is_err());
    }

    // ── Key validation ─────────────────────────────────────────────

    #[test]
    fn key_accepts_hex() {
        assert!(validate_key("abcdef1234567890", "public key").is_ok());
    }

    #[test]
    fn key_accepts_veilid_format() {
        assert!(validate_key("VLD0:abcdef:123456", "governance key").is_ok());
    }

    #[test]
    fn key_rejects_empty() {
        assert!(validate_key("", "key").is_err());
    }

    #[test]
    fn key_rejects_control_chars() {
        assert!(validate_key("abc\x00def", "key").is_err());
    }

    #[test]
    fn key_rejects_spaces() {
        assert!(validate_key("abc def", "key").is_err());
    }

    // ── Channel kind validation ────────────────────────────────────

    #[test]
    fn channel_kind_accepts_valid() {
        assert!(validate_channel_kind("text").is_ok());
        assert!(validate_channel_kind("voice").is_ok());
    }

    #[test]
    fn channel_kind_rejects_invalid() {
        assert!(validate_channel_kind("chat").is_err());
    }

    // ── Sanitization ───────────────────────────────────────────────

    #[test]
    fn sanitize_strips_ansi() {
        assert_eq!(sanitize_for_display("hello\x1b[31mworld"), "helloworld");
    }

    #[test]
    fn sanitize_strips_null() {
        assert_eq!(sanitize_for_display("hello\x00world"), "helloworld");
    }

    #[test]
    fn sanitize_preserves_newline() {
        assert_eq!(sanitize_for_display("a\nb"), "a\nb");
    }

    #[test]
    fn sanitize_preserves_unicode() {
        assert_eq!(sanitize_for_display("hello 🌍"), "hello 🌍");
    }
}
