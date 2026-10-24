//! A per-run bearer token for a loopback test bridge (the E2E server,
//! research batch 8 AL): 32 random bytes from the OS generator, spelled as
//! 52 characters of unpadded lowercase RFC 4648 base32, and checked in
//! constant time.

use rand::RngCore as _;
use subtle::ConstantTimeEq as _;

/// A minted token. Its `Debug` never shows the value.
pub struct BearerToken(String);

impl BearerToken {
    /// Mint a fresh token from the OS generator.
    #[must_use]
    pub fn mint() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(
            data_encoding::BASE32_NOPAD
                .encode(&bytes)
                .to_ascii_lowercase(),
        )
    }

    /// The token as it is handed to the client.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `presented` is this token, compared in constant time so the
    /// comparison does not reveal how much of a guess was right.
    #[must_use]
    pub fn matches(&self, presented: &[u8]) -> bool {
        bool::from(self.0.as_bytes().ct_eq(presented))
    }
}

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BearerToken(…)")
    }
}

#[cfg(test)]
mod tests {
    use super::BearerToken;

    #[test]
    fn minted_tokens_are_distinct_and_match_only_themselves() {
        let (a, b) = (BearerToken::mint(), BearerToken::mint());
        assert_eq!(a.as_str().len(), 52);
        assert_ne!(a.as_str(), b.as_str());
        assert!(a.matches(a.as_str().as_bytes()));
        assert!(!a.matches(b.as_str().as_bytes()));
        assert!(!a.matches(&a.as_str().as_bytes()[..51]));
        assert!(!format!("{a:?}").contains(a.as_str()));
    }
}
