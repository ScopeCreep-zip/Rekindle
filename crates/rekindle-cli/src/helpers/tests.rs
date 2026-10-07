use super::*;

// ── Sanitization: ANSI injection ─────────────────────────────────

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
