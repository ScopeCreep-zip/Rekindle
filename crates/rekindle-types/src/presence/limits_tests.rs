/// Plan E7.2: blank is no bio; 190 characters pass, 191 or a control
/// character do not.
#[test]
fn a_bio_is_blank_or_bounded_text() {
    assert_eq!(super::bio("  ").unwrap(), None);
    assert_eq!(super::bio("gg").unwrap(), Some("gg"));
    assert!(super::bio(&"🔥".repeat(super::MAX_BIO_LEN)).is_ok());
    assert!(super::bio(&"x".repeat(super::MAX_BIO_LEN + 1)).is_err());
    assert!(super::bio("a\u{7}b").is_err());
}

use super::*;

#[test]
fn validate_profile_accepts_empty_inputs() {
    assert!(validate_profile(None, None, &[], None, None).is_ok());
}

#[test]
fn validate_profile_accepts_boundary_inputs() {
    let bio = "x".repeat(MAX_BIO_LEN);
    let pronouns = "y".repeat(MAX_PRONOUNS_LEN);
    let badges: Vec<String> = (0..MAX_BADGES).map(|_| "z".repeat(MAX_BADGE_LEN)).collect();
    let avatar = "a".repeat(MAX_CONTENT_REF_LEN);
    let banner = "b".repeat(MAX_CONTENT_REF_LEN);
    assert!(validate_profile(
        Some(&bio),
        Some(&pronouns),
        &badges,
        Some(&avatar),
        Some(&banner)
    )
    .is_ok());
}

#[test]
fn validate_profile_rejects_oversized_bio() {
    let bio = "x".repeat(MAX_BIO_LEN + 1);
    assert!(validate_profile(Some(&bio), None, &[], None, None).is_err());
}

#[test]
fn validate_profile_rejects_oversized_pronouns() {
    let pronouns = "y".repeat(MAX_PRONOUNS_LEN + 1);
    assert!(validate_profile(None, Some(&pronouns), &[], None, None).is_err());
}

#[test]
fn validate_profile_rejects_too_many_badges() {
    let badges: Vec<String> = (0..=MAX_BADGES).map(|_| "a".to_string()).collect();
    assert!(validate_profile(None, None, &badges, None, None).is_err());
}

#[test]
fn validate_profile_rejects_oversized_badge() {
    let badges = vec!["z".repeat(MAX_BADGE_LEN + 1)];
    assert!(validate_profile(None, None, &badges, None, None).is_err());
}

/// Badges and content refs are identifiers: a 32-emoji badge would be
/// 128 bytes, four times what the row budget allows for it.
#[test]
fn identifiers_must_be_printable_ascii() {
    let emoji_badge = vec!["🔥".repeat(MAX_BADGE_LEN / 4)];
    assert!(validate_profile(None, None, &emoji_badge, None, None).is_err());
    let spaced = vec!["a b".to_string()];
    assert!(validate_profile(None, None, &spaced, None, None).is_err());
    assert!(validate_profile(None, None, &[], Some("ref\u{7f}"), None).is_err());
    assert!(validate_profile(None, None, &[String::new()], None, None).is_err());
}

#[test]
fn validate_profile_counts_unicode_chars_not_bytes() {
    // Each emoji is 4 bytes but 1 char. MAX_BIO_LEN emoji should pass.
    let bio: String = "🔥".repeat(MAX_BIO_LEN);
    assert!(validate_profile(Some(&bio), None, &[], None, None).is_ok());
    let bio_over: String = "🔥".repeat(MAX_BIO_LEN + 1);
    assert!(validate_profile(Some(&bio_over), None, &[], None, None).is_err());
}

#[test]
fn validate_profile_rejects_control_characters() {
    assert!(validate_profile(Some("a\u{0}b"), None, &[], None, None).is_err());
    assert!(validate_profile(None, Some("x\u{1b}"), &[], None, None).is_err());
}

#[test]
fn validate_profile_rejects_oversized_avatar_ref() {
    let oversized = "a".repeat(MAX_CONTENT_REF_LEN + 1);
    assert!(validate_profile(None, None, &[], Some(&oversized), None).is_err());
}

#[test]
fn display_name_is_bounded_and_trimmed() {
    assert_eq!(display_name("  Kali  "), Ok("Kali"));
    assert!(display_name(&"日".repeat(MAX_DISPLAY_NAME_LEN)).is_ok());
    assert!(display_name(&"a".repeat(MAX_DISPLAY_NAME_LEN + 1)).is_err());
    assert!(display_name("   ").is_err());
}
