//! Our identity rotation in flight (audit 2026-10-20 R-2).
//!
//! The rotation journals the new identity's secret and the seed its prekey
//! set is minted from before the set is published and before the first
//! `IdentityRotated` notice goes out: once a friend has heard of the new
//! key, losing it loses every friendship. The set is minted
//! deterministically from the seed, so the journal re-mints the very set
//! that was published. The entry lives in the old identity's vault and goes
//! with it once the registry names the new key; an unlock that finds it
//! completes the rotation instead of coming up under the old key.

use zeroize::Zeroizing;

use crate::{VaultError, VaultKey, VaultStore};

/// The secret (32) ‖ the mint seed (32) ‖ the set's issue time (8, LE).
const LEN: usize = 72;

/// A journalled rotation.
pub struct PendingRotation {
    /// The new identity's Ed25519 secret.
    pub secret: Zeroizing<[u8; 32]>,
    /// The seed the new identity's prekey set is minted from.
    pub mint_seed: Zeroizing<[u8; 32]>,
    /// The set's issue time (ms), covered by its signatures.
    pub issued_at_ms: i64,
}

impl std::fmt::Debug for PendingRotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingRotation")
            .field("issued_at_ms", &self.issued_at_ms)
            .finish_non_exhaustive()
    }
}

/// Journal `pending` (a write that fails is an error: no notice may go
/// out without it).
///
/// # Errors
/// The vault write failed.
pub fn persist(vault: &VaultStore, pending: &PendingRotation) -> Result<(), VaultError> {
    let mut bytes = Zeroizing::new([0u8; LEN]);
    bytes[..32].copy_from_slice(&*pending.secret);
    bytes[32..64].copy_from_slice(&*pending.mint_seed);
    bytes[64..].copy_from_slice(&pending.issued_at_ms.to_le_bytes());
    vault.put(&VaultKey::PendingRotation, &*bytes)
}

/// The journalled rotation, if one is in flight.
///
/// # Errors
/// The vault read failed or the entry is not the journal's shape.
pub fn load(vault: &VaultStore) -> Result<Option<PendingRotation>, VaultError> {
    let Some(stored) = vault.get(&VaultKey::PendingRotation)? else {
        return Ok(None);
    };
    if stored.len() != LEN {
        return Err(VaultError::Schema(
            "the pending rotation is not 72 bytes".into(),
        ));
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    secret.copy_from_slice(&stored[..32]);
    let mut mint_seed = Zeroizing::new([0u8; 32]);
    mint_seed.copy_from_slice(&stored[32..64]);
    let mut issued = [0u8; 8];
    issued.copy_from_slice(&stored[64..]);
    Ok(Some(PendingRotation {
        secret,
        mint_seed,
        issued_at_ms: i64::from_le_bytes(issued),
    }))
}

/// Drop the journal (the rotation was aborted before any notice, or this
/// vault is the new identity's copy).
///
/// # Errors
/// The vault write failed.
pub fn clear(vault: &VaultStore) -> Result<(), VaultError> {
    vault.delete(&VaultKey::PendingRotation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_round_trips_and_clears() {
        let dir = tempfile::TempDir::new().unwrap();
        let (vault, _) = VaultStore::create(&dir.path().join("a.vault"), "pass").unwrap();
        assert!(load(&vault).unwrap().is_none());
        let pending = PendingRotation {
            secret: Zeroizing::new([7; 32]),
            mint_seed: Zeroizing::new([9; 32]),
            issued_at_ms: 1234,
        };
        persist(&vault, &pending).unwrap();
        let back = load(&vault).unwrap().unwrap();
        assert_eq!(*back.secret, [7; 32]);
        assert_eq!(*back.mint_seed, [9; 32]);
        assert_eq!(back.issued_at_ms, 1234);
        clear(&vault).unwrap();
        assert!(load(&vault).unwrap().is_none());
    }
}
