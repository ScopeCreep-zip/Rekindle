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

/// Validate a display name as another party supplies it — a peer's own
/// name in their friend request, or the one a request names for them: at
/// most [`MAX_DISPLAY_NAME_LEN`] characters of [`key_format::text`] (no
/// control or hidden format characters; empty means none given). The IPC
/// validation and the 1:1 decoder both apply it (plan audit G6).
///
/// # Errors
/// The name is too long or holds a forbidden character.
pub fn peer_display_name(s: &str) -> Result<&str, KeyFormatError> {
    key_format::text(s, MAX_DISPLAY_NAME_LEN)
}

/// Our bio, or a friend's in its `ProfileBio` (plan E7.2): blank is no bio
/// (`None`); otherwise it is [`key_format::text`] within [`MAX_BIO_LEN`],
/// kept as written and shown as untrusted text.
///
/// # Errors
/// Over 190 characters, or a control character.
pub fn bio(s: &str) -> Result<Option<&str>, KeyFormatError> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    key_format::text(s, MAX_BIO_LEN).map(Some)
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
#[path = "limits_tests.rs"]
mod tests;
