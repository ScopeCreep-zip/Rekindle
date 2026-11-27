use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;

/// A user's cryptographic identity.
///
/// This is the foundation of Rekindle's identity system. There are no usernames
/// or passwords — identity IS the Ed25519 keypair. The public key is your address
/// on the network.
#[derive(ZeroizeOnDrop)]
pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    /// Generate a new random identity.
    pub fn generate() -> Self {
        Self::generate_with(&rekindle_utils::random::OsRandom)
    }

    /// A new random identity whose 32-byte secret is drawn from `rng`, the
    /// host's injected generator (plan C9.2).
    #[must_use]
    pub fn generate_with(rng: &dyn rekindle_utils::RandomSource) -> Self {
        let mut secret = zeroize::Zeroizing::new([0u8; 32]);
        rng.fill(secret.as_mut());
        Self::from_secret_bytes(&secret)
    }

    /// Restore an identity from a 32-byte secret key.
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(bytes);
        Self { signing_key }
    }

    /// Get the public verifying key (this is your "address" / identity).
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Get the public key as raw bytes (32 bytes).
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Get the secret key bytes (for storage in Stronghold).
    ///
    /// # Security
    /// Handle with care — this is the private key material.
    pub fn secret_key_bytes(&self) -> &[u8; 32] {
        self.signing_key.as_bytes()
    }

    /// Sign a message with this identity's private key.
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Verify a signature against a public key.
    pub fn verify(
        public_key: &VerifyingKey,
        message: &[u8],
        signature: &Signature,
    ) -> Result<(), CryptoError> {
        public_key
            .verify_strict(message, signature)
            .map_err(|e| CryptoError::verification(e.to_string()))
    }

    /// Derive an X25519 static secret from this Ed25519 key for Diffie-Hellman.
    ///
    /// Uses the SHA-512-expanded scalar (same scalar that Ed25519 uses internally)
    /// so that `to_x25519_public()` matches `peer_ed25519_to_x25519()` via the
    /// standard Edwards→Montgomery birational map.
    ///
    /// Used for Signal Protocol key agreement (X3DH).
    pub fn to_x25519_secret(&self) -> x25519_dalek::StaticSecret {
        let scalar_bytes = self.signing_key.to_scalar_bytes();
        x25519_dalek::StaticSecret::from(scalar_bytes)
    }

    /// Get the X25519 public key derived from this identity.
    pub fn to_x25519_public(&self) -> x25519_dalek::PublicKey {
        x25519_dalek::PublicKey::from(&self.to_x25519_secret())
    }

    /// Get the public key as a hex string (for display / sharing).
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }

    /// Convert a peer's Ed25519 public key bytes to an X25519 public key.
    ///
    /// Uses the standard Edwards→Montgomery birational map (RFC 7748).
    /// This is the correct way to derive X25519 from a **public** Ed25519 key
    /// (as opposed to `to_x25519_secret`/`to_x25519_public` which work from
    /// our own secret key).
    pub fn peer_ed25519_to_x25519(
        ed25519_public_bytes: &[u8; 32],
    ) -> Result<x25519_dalek::PublicKey, CryptoError> {
        let verifying_key = VerifyingKey::from_bytes(ed25519_public_bytes)
            .map_err(|e| CryptoError::verification(format!("invalid Ed25519 public key: {e}")))?;
        let montgomery = verifying_key.to_montgomery();
        Ok(x25519_dalek::PublicKey::from(montgomery.to_bytes()))
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("public_key", &self.public_key_hex())
            .finish()
    }
}

/// A public key as a person compares it out of band: its hex in groups
/// of four (plan U1.4: the subject of an admitting intent leads with our
/// nickname and this, never a peer-chosen name alone).
#[must_use]
pub fn key_fingerprint(public_key: &[u8]) -> String {
    hex::encode(public_key)
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A key's short fingerprint: the first eight hex digits of
/// [`key_fingerprint`], in its two groups of four (`"a1b2 c3d4"`). What a
/// community renders for a member whose card names nobody (plan E3.4 S6):
/// the identity's name is never written to a community.
#[must_use]
pub fn short_key_fingerprint(public_key: &[u8]) -> String {
    let full = key_fingerprint(public_key);
    full.get(..9).unwrap_or(&full).to_owned()
}

/// A DHT record key as a person compares it: [`key_fingerprint`] over the
/// record's opaque key bytes, from the key's text form
/// `KIND:base64url(opaque key)[:base64url(encryption key)]` (Veilid
/// `crypto/types/record_key.rs`). The encryption key, a secret, is never
/// part of it. `None` when the text is not a record key.
#[must_use]
pub fn record_key_fingerprint(record_key: &str) -> Option<String> {
    use base64::Engine;
    let (_kind, rest) = record_key.split_once(':')?;
    let opaque = rest.split(':').next()?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(opaque)
        .ok()?;
    (bytes.len() == 32).then(|| key_fingerprint(&bytes))
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
