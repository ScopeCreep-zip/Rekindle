//! Bounds on the self-authored fields of a presence row.
//!
//! The row lives in one 4112-byte registry slot (SMPL 255 subkeys, plan
//! V2), so every field it carries must have a bound, and the bound must
//! hold in bytes, not only in characters (plan C7.15). Free text is
//! counted in characters, as Discord and VeilidChat count it: at four
//! bytes per character the worst case still fits, which
//! `presence::tests::worst_case_row_fits_its_slot` proves. Identifiers
//! (badge ids, content refs) are ASCII, so their character count is their
//! byte count.
//!
//! One definition for both tracks: the desktop's profile command and the
//! daemon's IPC validation call these.

use crate::key_format::{self, KeyFormatError};

/// Longest display name.
pub const MAX_DISPLAY_NAME_LEN: usize = 64;
/// Longest bio: Discord's "About Me" (190 characters).
pub const MAX_BIO_LEN: usize = 190;
/// Architecture §24.2 specifies pronouns ≤40 chars.
pub const MAX_PRONOUNS_LEN: usize = 40;
/// Most badges one member shows.
pub const MAX_BADGES: usize = 8;
/// Longest badge id.
pub const MAX_BADGE_LEN: usize = 32;
/// blake3 content-hash hex (64 chars) is the canonical avatar/banner
/// reference per architecture §24.2; raw bytes never appear in the
/// `MemberPresence` record. The headroom is for a short scheme prefix.
pub const MAX_CONTENT_REF_LEN: usize = 96;

/// Validate a display name: non-empty after trimming, at most
/// [`MAX_DISPLAY_NAME_LEN`] characters, no control characters. Returns
/// the trimmed name.
///
/// # Errors
/// The name is empty, too long, or holds a control character.
pub fn display_name(s: &str) -> Result<&str, KeyFormatError> {
    key_format::name(s, MAX_DISPLAY_NAME_LEN)
}

/// Validate the self-authored profile fields a presence row carries.
///
/// # Errors
/// A field is over its bound, holds a control character, or an identifier
/// field (badge, content ref) is not printable ASCII.
pub fn validate_profile(
    bio: Option<&str>,
    pronouns: Option<&str>,
    badges: &[String],
    avatar_ref: Option<&str>,
    banner_ref: Option<&str>,
) -> Result<(), String> {
    if let Some(b) = bio {
        key_format::text(b, MAX_BIO_LEN).map_err(|e| format!("bio: {e}"))?;
    }
    if let Some(p) = pronouns {
        key_format::text(p, MAX_PRONOUNS_LEN).map_err(|e| format!("pronouns: {e}"))?;
    }
    if badges.len() > MAX_BADGES {
        return Err(format!("badges count exceeds {MAX_BADGES}"));
    }
    for badge in badges {
        identifier("badge", badge, MAX_BADGE_LEN)?;
    }
    if let Some(a) = avatar_ref {
        identifier("avatar_ref", a, MAX_CONTENT_REF_LEN)?;
    }
    if let Some(b) = banner_ref {
        identifier("banner_ref", b, MAX_CONTENT_REF_LEN)?;
    }
    Ok(())
}

/// An identifier: non-empty printable ASCII with no spaces, at most `max`
/// bytes.
fn identifier(label: &str, s: &str, max: usize) -> Result<(), String> {
    if s.is_empty() {
        return Err(format!("{label} is empty"));
    }
    if s.len() > max {
        return Err(format!("{label} exceeds {max} characters"));
    }
    if !s.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(format!("{label} must be printable ASCII"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
