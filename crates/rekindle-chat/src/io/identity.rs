//! Identity derivation — pseudonym keys, public key access.
//!
//! All derivations go through `SelfIdentity` from `rekindle-identity`.
//! No inline BLAKE3 domain tags, no raw seed access.

use super::PlatformIO;
use crate::ChatError;

impl PlatformIO {
    /// Derive the community pseudonym Ed25519 public key as hex string.
    ///
    /// Parses the governance key string to `GovernanceKey` which strips
    /// the VLD0: prefix via `canonical_bytes()`, then delegates to
    /// `SelfIdentity::pseudonym_seed()` with the frozen G3 derivation tag.
    pub fn pseudonym_hex(&self, community: &str) -> Result<String, ChatError> {
        self.with_identity(|si| {
            let gov = rekindle_identity::GovernanceKey::parse(community)
                .map_err(|e| ChatError::Internal(format!("governance key parse: {e}")))?;
            let seed = si.pseudonym_seed(&gov);
            let pseudo_kp = rekindle_identity::SigningKeypair::from_seed(&seed)
                .map_err(|e| ChatError::Internal(format!("pseudonym keypair: {e}")))?;
            Ok(hex::encode(pseudo_kp.public_key_bytes()))
        })
    }

    /// Derive the community pseudonym signing seed (32 bytes).
    ///
    /// Used by services that need to sign community-specific data.
    pub fn pseudonym_seed(&self, community: &str) -> Result<[u8; 32], ChatError> {
        self.with_identity(|si| {
            let gov = rekindle_identity::GovernanceKey::parse(community)
                .map_err(|e| ChatError::Internal(format!("governance key parse: {e}")))?;
            Ok(*si.pseudonym_seed(&gov))
        })
    }

    /// Get the identity Ed25519 public key as hex string.
    pub fn identity_public_key_hex(&self) -> Result<String, ChatError> {
        self.with_identity(|si| Ok(si.root().to_hex()))
    }

    /// Get the identity Ed25519 public key as 32 bytes.
    pub fn identity_public_key_bytes(&self) -> Result<[u8; 32], ChatError> {
        self.with_identity(|si| Ok(*si.root().as_bytes()))
    }

    /// Derive the X25519 DH seed from the identity signing key.
    ///
    /// Used for PQXDH handshakes and MEK wrapping ECDH.
    pub fn x25519_seed(&self) -> Result<[u8; 32], ChatError> {
        self.with_identity(|si| Ok(*si.x25519_identity_seed()))
    }
}
