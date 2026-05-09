//! Vault unlock mechanisms — passphrase, OS keyring, auto-keyring, SSH agent.
//!
//! Each implementation recovers the 32-byte master key using its own
//! authentication flow. The master key is returned in [`MasterKey`]
//! (`Zeroizing<[u8; 32]>`, fixed-size, no Copy, zeroize-on-drop).

pub mod passphrase;
pub mod keyring;
pub mod auto_keyring;
pub mod ssh;
pub mod methods;

use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::error::StorageResult;

/// The 32-byte master key. Fixed-size (no `Vec` reallocation), zeroized
/// on drop. This type is the root of all key
/// derivation in the vault — vault_key, entry_key, audit_key, and
/// session_mac_key are all BLAKE3-derived from this.
#[derive(ZeroizeOnDrop)]
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl MasterKey {
    /// Generate a random 32-byte master key.
    pub fn generate() -> StorageResult<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        aws_lc_rs::rand::SystemRandom::new()
            .fill(bytes.as_mut())
            .map_err(|e| crate::error::StorageError::RngFailed(format!("{e}")))?;
        Ok(Self(bytes))
    }

    /// Construct from existing bytes (e.g., after unwrapping from keyring).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Access the raw bytes. Lifetime bounded by the `MasterKey` — callers
    /// cannot outlive the unlock period.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey([REDACTED])")
    }
}

use aws_lc_rs::rand::SecureRandom;

/// Abstraction over vault unlock mechanisms.
pub trait VaultUnlock: Send + Sync {
    /// Attempt to recover the master key.
    fn unlock(&self) -> StorageResult<MasterKey>;

    /// Store the master key for future unlocks (wrapping with KEK).
    fn enroll(&self, master_key: &MasterKey) -> StorageResult<()>;

    /// Remove this unlock method's stored material.
    fn revoke(&self) -> StorageResult<()>;

    /// Check if this unlock method is currently available on this platform.
    fn is_available(&self) -> bool;

    /// Unique identifier for this method (used in `unlock_methods.json`).
    fn id(&self) -> &'static str;

    /// Human-readable name for UI display.
    fn display_name(&self) -> &'static str;
}
