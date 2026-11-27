//! The canonical static test fixtures (owner ruling 2026-11-26: "identify our regular test
//! identities and items and make sure they're static so that we can make detection rules to be
//! mindful of them").
//!
//! Every test value a secret scanner could read as a credential is defined here once and
//! referenced by path (`rekindle_test_fixtures::HEX_KEY_64`); no test keeps its own copy.
//! `cargo xtask check-test-fixtures` fails on a secret-shaped literal in test code, and
//! `.gitleaks.toml` allowlists exactly these values (anchored) in crate source and goldens.
//! The crate is a dev-dependency only; nothing shipped links it.
//!
//! **The marker (synthetic by construction).** Every secret-shaped fixture is built from a
//! documented *ramp*, the in-order run of its alphabet:
//!
//! - hex values repeat [`HEX_RAMP`] (`abcdef1234567890`) end to end, so a 32-byte key repeats
//!   its first 8 bytes four times; a random key does that with probability 2^-192;
//! - base32 tokens begin with [`BASE32_RAMP`] (`abcdefghijklmnopqrstuvwx`), 120 bits a random
//!   token matches with probability 2^-120; the last two characters finish a valid 128-bit
//!   RFC 4648 base32 encoding.
//!
//! Passphrases and display names are low-entropy and never read as credentials; the ones tests
//! share are named here so they stay static too.
//!
//! **Published known-answer vectors** ([`kat`]) cannot carry the marker: the standard fixes their
//! bytes. A key-shaped one is kept here once, named after the document and section that publish
//! it, so a scanner hit on it is checked against that citation.

/// The 16-digit block every hex fixture repeats (the marker for hex values).
pub const HEX_RAMP: &str = "abcdef1234567890";

/// A 32-byte key as 64 hex digits: [`HEX_RAMP`] four times. Also the value gitleaks flagged in
/// history (`let key = …` in `rekindle-client/src/fmt.rs` and the `rekindle-cli` helpers tests).
pub const HEX_KEY_64: &str = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

/// The 24-character prefix every base32 token fixture starts with (the marker for tokens).
pub const BASE32_RAMP: &str = "abcdefghijklmnopqrstuvwx";

/// A friend-code token (16 bytes, 26 base32 characters): [`BASE32_RAMP`] then `2q`. Also the
/// value gitleaks flagged in history (`token` in `rekindle-types/tests/golden/dto.json`).
pub const FRIEND_TOKEN: &str = "abcdefghijklmnopqrstuvwx2q";

/// The identity passphrase vault, unlock and intent tests share.
pub const PASSPHRASE: &str = "pass";

/// The spoken-style passphrase wire, serde and redaction tests share.
pub const PASSPHRASE_SPOKEN: &str = "correct horse";

/// Key-shaped known-answer vectors from published standards, cited by document and section.
pub mod kat {
    /// NIST SP 800-38A Appendix F.2.5 (CBC-AES256.Encrypt): the AES-256 key.
    pub const NIST_SP800_38A_AES256_KEY: &str =
        "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4";
}

#[cfg(test)]
mod tests {
    use super::{BASE32_RAMP, FRIEND_TOKEN, HEX_KEY_64, HEX_RAMP};

    #[test]
    fn every_secret_shaped_fixture_carries_its_marker() {
        assert_eq!(HEX_KEY_64, HEX_RAMP.repeat(4));
        assert!(HEX_KEY_64.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(FRIEND_TOKEN.len(), 26);
        assert!(FRIEND_TOKEN.starts_with(BASE32_RAMP));
        assert!(FRIEND_TOKEN
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)));
    }
}
