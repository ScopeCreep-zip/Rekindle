//! The unlocked identity's Ed25519 signing key, held in memory between
//! unlock and lock.
//!
//! [RC-16] Zeroized on drop, including during panics.

use zeroize::ZeroizeOnDrop;

/// In-memory handle to the Ed25519 signing key. The daemon holds exactly
/// one after unlock (from `host::identity::unlock`) and drops it on lock or
/// shutdown.
#[derive(ZeroizeOnDrop)]
pub struct SigningKeyHandle {
    #[zeroize]
    bytes: [u8; 32],
}

impl SigningKeyHandle {
    /// Hold `bytes` (the caller's copy is its own to zeroize).
    #[must_use]
    pub fn new(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// Access the raw signing key bytes.
    ///
    /// Only called by transport operations that need to sign payloads.
    /// The reference lifetime is bounded by the handle — callers cannot
    /// outlive the unlock period.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl std::fmt::Debug for SigningKeyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKeyHandle")
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}
