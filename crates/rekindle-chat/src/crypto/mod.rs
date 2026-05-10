//! Cryptographic services for the chat layer.
//!
//! Wraps `rekindle-ratchet` primitives and manages session cache +
//! MEK cache + signing key handle. No raw crypto operations outside
//! this module — all other chat modules call these functions.

pub mod sessions;
pub mod envelope;
pub mod mek;

use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::ChatError;

/// In-memory handle to the Ed25519 signing key seed.
/// Loaded from vault on unlock, zeroized on lock/drop.
#[derive(ZeroizeOnDrop)]
pub struct SigningKeyHandle {
    seed: Zeroizing<[u8; 32]>,
}

impl SigningKeyHandle {
    /// Load the signing key from the vault.
    pub fn from_vault(vault: &rekindle_storage::VaultStore) -> Result<Self, ChatError> {
        let bytes = vault.require_key(rekindle_storage::keys::labels::SIGNING_KEY)?;
        if bytes.len() != 32 {
            return Err(ChatError::Internal(format!(
                "signing key wrong length: {} (expected 32)",
                bytes.len()
            )));
        }
        let mut seed = Zeroizing::new([0u8; 32]);
        seed.copy_from_slice(&bytes);
        Ok(Self { seed })
    }

    /// Raw seed bytes. Lifetime bounded by the handle.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.seed
    }

    /// Derive the community pseudonym signing key seed.
    pub fn pseudonym_seed(&self, community_id: &str) -> [u8; 32] {
        blake3::derive_key(
            &format!("rekindle pseudonym v1 {community_id}"),
            self.seed.as_ref(),
        )
    }

    /// Derive the X25519 DH identity seed (separate from Ed25519).
    pub fn x25519_seed(&self) -> [u8; 32] {
        blake3::derive_key("rekindle identity x25519 v1", self.seed.as_ref())
    }

    /// Ed25519 public key as 32 bytes.
    pub fn public_key_bytes(&self) -> Result<[u8; 32], ChatError> {
        let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&self.seed)
            .map_err(|e| ChatError::Internal(format!("keypair from seed: {e}")))?;
        Ok(rekindle_ratchet::crypto::sign::public_key_bytes(&kp))
    }

    /// Ed25519 public key as hex string.
    pub fn public_key_hex(&self) -> Result<String, ChatError> {
        Ok(hex::encode(self.public_key_bytes()?))
    }
}

impl std::fmt::Debug for SigningKeyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKeyHandle([REDACTED])")
    }
}
