//! Group-DM sending keys (plan E2.6): each member of a group DM holds a
//! random 32-byte key of its own and sends it to every other member over
//! their pairwise session. A member's `GroupContent` is sealed under its
//! own key of the frame's epoch; on any leave or block every remaining
//! member mints the next epoch, so a departed member's copies open
//! nothing written after it.
//!
//! The sealed form is `nonce(12) ‖ AES-256-GCM(ct ‖ tag)`, with associated
//! data [`domains::GROUP_DM_CONTENT`] `‖ group_id(16) ‖ u32be(epoch) ‖
//! sender`: a ciphertext is bound to its group, its epoch and its sender,
//! so one member's entry cannot be replayed as another's, into another
//! group, or across a rotation. The epoch travels in the clear beside it
//! (`GroupContent.epoch`). The nonce is random (`OsRng`); one key seals at
//! most one member's messages for one epoch, far below GCM's random-nonce
//! bound. A value that does not open is dropped; there is no plaintext form.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use rekindle_utils::RandomSource;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The key's length.
pub const SENDER_KEY_LEN: usize = 32;
/// A group id's length.
pub const GROUP_ID_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// One epoch of one member's group sending key.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct GroupSenderKey {
    epoch: u32,
    key: [u8; SENDER_KEY_LEN],
}

/// The key is never printed.
impl std::fmt::Debug for GroupSenderKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupSenderKey")
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

impl GroupSenderKey {
    /// A key as stored or received.
    #[must_use]
    pub fn new(epoch: u32, key: [u8; SENDER_KEY_LEN]) -> Self {
        Self { epoch, key }
    }

    /// Mint epoch `epoch` from the engine's generator.
    #[must_use]
    pub fn mint(epoch: u32, rng: &dyn RandomSource) -> Self {
        let mut key = [0u8; SENDER_KEY_LEN];
        rng.fill(&mut key);
        let minted = Self { epoch, key };
        key.zeroize();
        minted
    }

    /// The epoch after this one, freshly minted; `None` when the epochs
    /// are exhausted.
    #[must_use]
    pub fn next(&self, rng: &dyn RandomSource) -> Option<Self> {
        Some(Self::mint(self.epoch.checked_add(1)?, rng))
    }

    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; SENDER_KEY_LEN] {
        &self.key
    }
}

fn aad(group_id: &[u8; GROUP_ID_LEN], epoch: u32, sender: &str) -> Vec<u8> {
    let label = domains::GROUP_DM_CONTENT.as_bytes();
    let mut aad = Vec::with_capacity(label.len() + GROUP_ID_LEN + 4 + sender.len());
    aad.extend_from_slice(label);
    aad.extend_from_slice(group_id);
    aad.extend_from_slice(&epoch.to_be_bytes());
    aad.extend_from_slice(sender.as_bytes());
    aad
}

/// Seal `plaintext` as `sender`'s entry in `group_id` under `key` (its
/// epoch is the entry's).
///
/// # Errors
/// The cipher failed.
pub fn seal(
    key: &GroupSenderKey,
    group_id: &[u8; GROUP_ID_LEN],
    sender: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher =
        Aes256Gcm::new_from_slice(&key.key).map_err(|e| CryptoError::Encryption(e.to_string()))?;
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad(group_id, key.epoch, sender),
            },
        )
        .map_err(|e| CryptoError::Encryption(e.to_string()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open `sender`'s entry in `group_id` that [`seal`] made under `key`.
///
/// # Errors
/// It is too short or does not authenticate (another key, epoch, group or
/// sender).
pub fn open(
    key: &GroupSenderKey,
    group_id: &[u8; GROUP_ID_LEN],
    sender: &str,
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::Decryption("group content too short".into()));
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    let cipher =
        Aes256Gcm::new_from_slice(&key.key).map_err(|e| CryptoError::Decryption(e.to_string()))?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: &aad(group_id, key.epoch, sender),
            },
        )
        .map_err(|_| CryptoError::Decryption("group content does not open".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_utils::OsRandom;

    const GROUP: [u8; GROUP_ID_LEN] = [7; GROUP_ID_LEN];

    #[test]
    fn round_trip() {
        let key = GroupSenderKey::mint(0, &OsRandom);
        let sealed = seal(&key, &GROUP, "ann", b"hi").expect("seal");
        assert_eq!(open(&key, &GROUP, "ann", &sealed).expect("open"), b"hi");
    }

    #[test]
    fn bound_to_group_sender_and_epoch() {
        let key = GroupSenderKey::mint(3, &OsRandom);
        let sealed = seal(&key, &GROUP, "ann", b"hi").expect("seal");
        assert!(open(&key, &[8; GROUP_ID_LEN], "ann", &sealed).is_err());
        assert!(open(&key, &GROUP, "bea", &sealed).is_err());
        let same_bytes_other_epoch = GroupSenderKey::new(4, *key.as_bytes());
        assert!(open(&same_bytes_other_epoch, &GROUP, "ann", &sealed).is_err());
        let next = key.next(&OsRandom).expect("next");
        assert_eq!(next.epoch(), 4);
        assert!(open(&next, &GROUP, "ann", &sealed).is_err());
        assert!(open(&key, &GROUP, "ann", &sealed[..20]).is_err());
    }

    #[test]
    fn epochs_end() {
        assert!(GroupSenderKey::mint(u32::MAX, &OsRandom)
            .next(&OsRandom)
            .is_none());
    }

    #[test]
    fn debug_redacts() {
        let key = GroupSenderKey::new(1, [0xAB; SENDER_KEY_LEN]);
        assert!(!format!("{key:?}").contains("171"));
    }
}
