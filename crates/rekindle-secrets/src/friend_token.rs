//! The friend-request token (plan E2.5, D17): 16 random bytes, spelled as
//! 26 characters of unpadded lowercase RFC 4648 base32, carried by our
//! friend code and checked against every incoming `FriendRequest`.

use rand::RngCore as _;
use rekindle_types::key_format::{self, FriendRequestToken};
use subtle::ConstantTimeEq as _;

/// Mint a fresh token from the OS generator, like the identity key it is
/// created with.
#[must_use]
pub fn mint() -> FriendRequestToken {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let spelled = data_encoding::BASE32_NOPAD
        .encode(&bytes)
        .to_ascii_lowercase();
    match key_format::friend_request_token(&spelled) {
        Ok(token) => token,
        // 16 bytes always spell 26 base32 characters with zero padding bits.
        Err(e) => unreachable!("base32 of 16 bytes is a token: {e}"),
    }
}

/// Whether `presented` is our token, compared in constant time so the
/// comparison does not reveal how much of a guess was right.
#[must_use]
pub fn matches(ours: &FriendRequestToken, presented: &str) -> bool {
    bool::from(ours.as_str().as_bytes().ct_eq(presented.as_bytes()))
}

/// The derivation context of a friend code's request-inbox writer (plan
/// RV2e).
const INBOX_WRITER_CONTEXT: &str = "rekindle friend-code request inbox writer v1";

/// The one-time writer of our friend code's request inbox (plan RV2e): an
/// Ed25519 key derived from the code's token, so exactly the code's holders
/// can write the inbox, and rotating the token changes the writer. The code
/// itself carries nothing more.
#[must_use]
pub fn inbox_writer(token: &FriendRequestToken) -> ed25519_dalek::SigningKey {
    let seed = zeroize::Zeroizing::new(blake3::derive_key(
        INBOX_WRITER_CONTEXT,
        token.as_str().as_bytes(),
    ));
    ed25519_dalek::SigningKey::from_bytes(&seed)
}

#[cfg(test)]
mod tests {
    use super::{inbox_writer, matches, mint};

    #[test]
    fn the_inbox_writer_follows_the_token() {
        let (a, b) = (mint(), mint());
        assert_eq!(inbox_writer(&a).to_bytes(), inbox_writer(&a).to_bytes());
        assert_ne!(inbox_writer(&a).to_bytes(), inbox_writer(&b).to_bytes());
    }

    #[test]
    fn minted_tokens_are_valid_distinct_and_compare() {
        let (a, b) = (mint(), mint());
        assert_ne!(a, b);
        assert!(matches(&a, a.as_str()));
        assert!(!matches(&a, b.as_str()));
        assert!(!matches(&a, ""));
    }
}
