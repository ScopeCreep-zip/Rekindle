//! Friend presence (plan E2.5): our profile's STATUS (subkey 2) and
//! GAME_INFO (subkey 4) are sealed under a 32-byte friends key that only
//! our accepted friends hold (sent to each as a ratcheted `FriendsKey`).
//! The profile record is public on the DHT; without this anyone who learns
//! its key reads when we are online and what we play.
//!
//! A new epoch is minted on every unfriend or block, so a removed peer's
//! copy opens nothing written after it. The sealed form is
//! `u32be(epoch) ‖ nonce(12) ‖ AES-256-GCM(ct ‖ tag)`, with associated data
//! [`domains::FRIEND_PRESENCE`] `‖ u32be(epoch) ‖ u32be(subkey) ‖ owner`:
//! a value is bound to its owner, its subkey and its epoch, so one friend's
//! value cannot be replayed as another's, a game as a status, or an old
//! epoch's value under the new key. A value that does not open is dropped;
//! there is no plaintext form.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use rekindle_utils::RandomSource;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The key's length.
pub const PRESENCE_KEY_LEN: usize = 32;
const EPOCH_LEN: usize = 4;
const NONCE_LEN: usize = 12;

/// One epoch of a friends key: ours (the newest we minted) or a friend's
/// (the newest they sent us).
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct PresenceKey {
    epoch: u32,
    key: [u8; PRESENCE_KEY_LEN],
}

/// The key is never printed.
impl std::fmt::Debug for PresenceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresenceKey")
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

impl PresenceKey {
    /// A key as stored or received.
    #[must_use]
    pub fn new(epoch: u32, key: [u8; PRESENCE_KEY_LEN]) -> Self {
        Self { epoch, key }
    }

    /// A stored key; `None` when the bytes are not 32 long.
    #[must_use]
    pub fn from_stored(epoch: u32, key: &[u8]) -> Option<Self> {
        Some(Self::new(epoch, key.try_into().ok()?))
    }

    /// Mint epoch `epoch` from the engine's generator.
    #[must_use]
    pub fn mint(epoch: u32, rng: &dyn RandomSource) -> Self {
        let mut key = [0u8; PRESENCE_KEY_LEN];
        rng.fill(&mut key);
        Self { epoch, key }
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
    pub fn as_bytes(&self) -> &[u8; PRESENCE_KEY_LEN] {
        &self.key
    }
}

fn aad(owner: &str, subkey: u32, epoch: u32) -> Vec<u8> {
    let label = domains::FRIEND_PRESENCE.as_bytes();
    let mut aad = Vec::with_capacity(label.len() + 8 + owner.len());
    aad.extend_from_slice(label);
    aad.extend_from_slice(&epoch.to_be_bytes());
    aad.extend_from_slice(&subkey.to_be_bytes());
    aad.extend_from_slice(owner.as_bytes());
    aad
}

/// Seal `plaintext` for `owner`'s profile subkey `subkey`. The nonce is
/// from `OsRng`, as every AEAD nonce here.
///
/// # Errors
/// The cipher failed.
pub fn seal(
    key: &PresenceKey,
    owner: &str,
    subkey: u32,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher =
        Aes256Gcm::new_from_slice(&key.key).map_err(|e| CryptoError::Encryption(e.to_string()))?;
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let aad = aad(owner, subkey, key.epoch);
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|e| CryptoError::Encryption(e.to_string()))?;
    let mut out = Vec::with_capacity(EPOCH_LEN + NONCE_LEN + ct.len());
    out.extend_from_slice(&key.epoch.to_be_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a value [`seal`] made for `owner`'s subkey `subkey`.
///
/// # Errors
/// It is too short, of another epoch, or does not authenticate.
pub fn open(
    key: &PresenceKey,
    owner: &str,
    subkey: u32,
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < EPOCH_LEN + NONCE_LEN {
        return Err(CryptoError::Decryption("presence value too short".into()));
    }
    let (epoch, rest) = sealed.split_at(EPOCH_LEN);
    let epoch = u32::from_be_bytes(
        epoch
            .try_into()
            .map_err(|_| CryptoError::Decryption("presence epoch".into()))?,
    );
    if epoch != key.epoch {
        return Err(CryptoError::Decryption(format!(
            "presence value of epoch {epoch}, key of epoch {}",
            key.epoch
        )));
    }
    let (nonce, ct) = rest.split_at(NONCE_LEN);
    let cipher =
        Aes256Gcm::new_from_slice(&key.key).map_err(|e| CryptoError::Decryption(e.to_string()))?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: &aad(owner, subkey, epoch),
            },
        )
        .map_err(|e| CryptoError::Decryption(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{open, seal, PresenceKey};

    const OWNER: &str = "aa11";

    #[test]
    fn a_sealed_value_opens_with_its_key_only() {
        let key = PresenceKey::new(3, [7; 32]);
        let sealed = seal(&key, OWNER, 2, b"\x01status").unwrap();
        assert_eq!(open(&key, OWNER, 2, &sealed).unwrap(), b"\x01status");
        // Another key of the same epoch, the next epoch, another subkey,
        // another owner, a flipped bit: none opens.
        assert!(open(&PresenceKey::new(3, [8; 32]), OWNER, 2, &sealed).is_err());
        assert!(open(&PresenceKey::new(4, [7; 32]), OWNER, 2, &sealed).is_err());
        assert!(open(&key, OWNER, 4, &sealed).is_err());
        assert!(open(&key, "bb22", 2, &sealed).is_err());
        let mut flipped = sealed.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert!(open(&key, OWNER, 2, &flipped).is_err());
        // A plaintext value is never shown.
        assert!(open(&key, OWNER, 2, &[0]).is_err());
        assert!(open(&key, OWNER, 2, &[]).is_err());
    }

    #[test]
    fn the_next_epoch_is_a_new_key() {
        let rng = rekindle_utils::OsRandom;
        let first = PresenceKey::mint(1, &rng);
        let next = first.next(&rng).unwrap();
        assert_eq!(next.epoch(), 2);
        assert_ne!(next.as_bytes(), first.as_bytes());
        assert!(PresenceKey::new(u32::MAX, [0; 32]).next(&rng).is_none());
        assert!(!format!("{first:?}").contains('7'));
    }
}
