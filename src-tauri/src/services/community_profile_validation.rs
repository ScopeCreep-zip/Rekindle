//! Phase 23.C — pure profile-field validators lifted from
//! `commands/community/presence.rs`. The MAX_* constants + the
//! `validate_profile` predicate + its full unit-test suite live
//! here as a pure module (no AppState, no Veilid, no SQL).

pub const MAX_BIO_LEN: usize = 190;
/// Architecture §24.2 specifies pronouns ≤40 chars.
pub const MAX_PRONOUNS_LEN: usize = 40;
pub const MAX_BADGES: usize = 8;
pub const MAX_BADGE_LEN: usize = 32;
/// blake3 content-hash hex (64 chars) is the canonical avatar/banner
/// reference per architecture §24.2; raw bytes never appear in the
/// MemberPresence record. Anything beyond hex+small-prefix scheme
/// pollutes the DHT subkey and bloats presence updates.
pub const MAX_CONTENT_REF_LEN: usize = 96;

pub fn validate_profile(
    bio: Option<&str>,
    pronouns: Option<&str>,
    badges: &[String],
    avatar_ref: Option<&str>,
    banner_ref: Option<&str>,
) -> Result<(), String> {
    if let Some(b) = bio {
        if b.chars().count() > MAX_BIO_LEN {
            return Err(format!("bio exceeds {MAX_BIO_LEN} characters"));
        }
    }
    if let Some(p) = pronouns {
        if p.chars().count() > MAX_PRONOUNS_LEN {
            return Err(format!("pronouns exceeds {MAX_PRONOUNS_LEN} characters"));
        }
    }
    if badges.len() > MAX_BADGES {
        return Err(format!("badges count exceeds {MAX_BADGES}"));
    }
    if badges.iter().any(|b| b.chars().count() > MAX_BADGE_LEN) {
        return Err(format!("badge exceeds {MAX_BADGE_LEN} characters"));
    }
    if let Some(a) = avatar_ref {
        if a.chars().count() > MAX_CONTENT_REF_LEN {
            return Err(format!("avatar_ref exceeds {MAX_CONTENT_REF_LEN} characters"));
        }
    }
    if let Some(b) = banner_ref {
        if b.chars().count() > MAX_CONTENT_REF_LEN {
            return Err(format!("banner_ref exceeds {MAX_CONTENT_REF_LEN} characters"));
        }
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
        let badges: Vec<String> = (0..MAX_BADGES)
            .map(|_| "z".repeat(MAX_BADGE_LEN))
            .collect();
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

    #[test]
    fn validate_profile_counts_unicode_chars_not_bytes() {
        // Each emoji is 4 bytes but 1 char. MAX_BIO_LEN emoji should pass.
        let bio: String = "🔥".repeat(MAX_BIO_LEN);
        assert!(validate_profile(Some(&bio), None, &[], None, None).is_ok());
        // One extra emoji should fail.
        let bio_over: String = "🔥".repeat(MAX_BIO_LEN + 1);
        assert!(validate_profile(Some(&bio_over), None, &[], None, None).is_err());
    }

    #[test]
    fn validate_profile_pronouns_now_allow_40_chars() {
        // Architecture §24.2 raised the cap from 32 to 40.
        let pronouns = "y".repeat(40);
        assert!(validate_profile(None, Some(&pronouns), &[], None, None).is_ok());
        let too_long = "y".repeat(41);
        assert!(validate_profile(None, Some(&too_long), &[], None, None).is_err());
    }

    #[test]
    fn validate_profile_rejects_oversized_avatar_ref() {
        let oversized = "a".repeat(MAX_CONTENT_REF_LEN + 1);
        assert!(validate_profile(None, None, &[], Some(&oversized), None).is_err());
    }
}
