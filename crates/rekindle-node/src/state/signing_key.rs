//! The unlocked identity's Ed25519 signing key, held in memory between
//! unlock and lock.
//!
//! [RC-16] Zeroized on drop, including during panics.

use rekindle_secrets::locked::LockedBox;
use zeroize::Zeroize;

/// In-memory handle to the Ed25519 signing key. The daemon holds exactly
/// one after unlock (from `host::identity::unlock`) and drops it on lock or
/// shutdown. The key sits in locked memory while held and is zeroized on
/// drop (plan E7.1, roadmap 5.2).
pub struct SigningKeyHandle {
    bytes: LockedBox<32>,
}

/// Dropping the handle drops its `LockedBox`, which zeroizes.
impl zeroize::ZeroizeOnDrop for SigningKeyHandle {}

impl SigningKeyHandle {
    /// Hold `bytes`; the copy passed in is zeroized here (the caller's own
    /// copy is its own to zeroize).
    #[must_use]
    pub fn new(mut bytes: [u8; 32]) -> Self {
        let handle = Self {
            bytes: LockedBox::from_bytes(&bytes),
        };
        bytes.zeroize();
        handle
    }

    /// Access the raw signing key bytes.
    ///
    /// Only called by transport operations that need to sign payloads.
    /// The reference lifetime is bounded by the handle — callers cannot
    /// outlive the unlock period.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.bytes.expose()
    }

    /// Whether the OS locked the key's pages (false when the lock budget
    /// refused; the key still signs).
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.bytes.is_locked()
    }
}

impl std::fmt::Debug for SigningKeyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKeyHandle(<redacted>)")
    }
}
