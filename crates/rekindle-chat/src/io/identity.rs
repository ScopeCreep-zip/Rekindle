//! Identity derivation — pseudonym keys, public key access.
//!
//! Deterministic key derivation from the signing key seed. Same inputs
//! always produce the same output — no storage needed, just re-derive.

use super::PlatformIO;
use crate::ChatError;

impl PlatformIO {
    /// Derive the community pseudonym Ed25519 public key as hex string.
    ///
    /// Deterministic from signing_key + community governance key. The same
    /// user in the same community always has the same pseudonym. Different
    /// communities produce different pseudonyms — no cross-community
    /// identity correlation.
    pub fn pseudonym_hex(&self, community: &str) -> Result<String, ChatError> {
        self.with_signing_key(|sk| {
            let seed = sk.pseudonym_seed(community);
            let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&seed)
                .map_err(|e| ChatError::Internal(format!("pseudonym keypair: {e}")))?;
            Ok(hex::encode(rekindle_ratchet::crypto::sign::public_key_bytes(&kp)))
        })
    }

    /// Derive the community pseudonym signing seed (32 bytes).
    ///
    /// Used by services that need to sign community-specific data beyond
    /// what PlatformIO's envelope methods cover (e.g., join request
    /// signatures, governance operation signatures).
    pub fn pseudonym_seed(&self, community: &str) -> Result<[u8; 32], ChatError> {
        self.with_signing_key(|sk| Ok(sk.pseudonym_seed(community)))
    }

    /// Get the identity Ed25519 public key as hex string.
    ///
    /// This is the user's global identity — used in DM addressing, friend
    /// requests, and profile DHT publication. Not community-specific.
    pub fn identity_public_key_hex(&self) -> Result<String, ChatError> {
        self.with_signing_key(crate::crypto::SigningKeyHandle::public_key_hex)
    }

    /// Get the identity Ed25519 public key as 32 bytes.
    pub fn identity_public_key_bytes(&self) -> Result<[u8; 32], ChatError> {
        self.with_signing_key(crate::crypto::SigningKeyHandle::public_key_bytes)
    }

    /// Derive the X25519 DH seed from the identity signing key.
    ///
    /// Used for PQXDH handshakes and MEK wrapping ECDH. Separate from
    /// the Ed25519 signing seed — no scalar reuse between signing and DH.
    pub fn x25519_seed(&self) -> Result<[u8; 32], ChatError> {
        self.with_signing_key(|sk| Ok(sk.x25519_seed()))
    }
}
