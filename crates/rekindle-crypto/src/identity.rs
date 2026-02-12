use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;

/// A user's cryptographic identity.
///
/// This is the foundation of Rekindle's identity system. There are no usernames
/// or passwords — identity IS the Ed25519 keypair. The public key is your address
/// on the network.
#[derive(ZeroizeOnDrop)]
pub struct Identity {
    #[zeroize(skip)]
    signing_key: SigningKey,
}

impl Identity {
    /// Generate a new random identity.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self { signing_key }
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
            .verify(message, signature)
            .map_err(|e| CryptoError::VerificationError(e.to_string()))
    }

    /// Derive an X25519 static secret from this Ed25519 key for Diffie-Hellman.
    ///
    /// Used for Signal Protocol key agreement (X3DH).
    pub fn to_x25519_secret(&self) -> x25519_dalek::StaticSecret {
        // Ed25519 secret key bytes can be clamped and used as X25519 secret
        let secret_bytes = self.signing_key.to_bytes();
        x25519_dalek::StaticSecret::from(secret_bytes)
    }

    /// Get the X25519 public key derived from this identity.
    pub fn to_x25519_public(&self) -> x25519_dalek::PublicKey {
        x25519_dalek::PublicKey::from(&self.to_x25519_secret())
    }

    /// Get the public key as a hex string (for display / sharing).
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("public_key", &self.public_key_hex())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_sign() {
        let identity = Identity::generate();
        let message = b"hello rekindle";

        let signature = identity.sign(message);
        assert!(Identity::verify(&identity.public_key(), message, &signature).is_ok());
    }

    #[test]
    fn roundtrip_secret_bytes() {
        let identity = Identity::generate();
        let bytes = *identity.secret_key_bytes();
        let restored = Identity::from_secret_bytes(&bytes);
        assert_eq!(identity.public_key_bytes(), restored.public_key_bytes());
    }

    #[test]
    fn x25519_derivation() {
        let alice = Identity::generate();
        let bob = Identity::generate();

        let alice_secret = alice.to_x25519_secret();
        let bob_public = bob.to_x25519_public();
        let bob_secret = bob.to_x25519_secret();
        let alice_public = alice.to_x25519_public();

        let shared_a = alice_secret.diffie_hellman(&bob_public);
        let shared_b = bob_secret.diffie_hellman(&alice_public);

        assert_eq!(shared_a.as_bytes(), shared_b.as_bytes());
    }
}
