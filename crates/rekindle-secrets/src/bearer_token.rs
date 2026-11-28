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

    /// A token minted by another process for this run (the WebDriver walk's
    /// runner hands it to the app it launches): `None` unless `spelled` is
    /// exactly what [`BearerToken::mint`] produces, 52 characters of
    /// lowercase unpadded base32 that decode to 32 bytes.
    #[must_use]
    pub fn parse(spelled: &str) -> Option<Self> {
        let upper = spelled.to_ascii_uppercase();
        let lowercase = spelled.bytes().all(|b| !b.is_ascii_uppercase());
        let decoded = data_encoding::BASE32_NOPAD.decode(upper.as_bytes()).ok()?;
        (lowercase && decoded.len() == 32).then(|| Self(spelled.to_owned()))
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
#[path = "bearer_token_tests.rs"]
mod tests;
