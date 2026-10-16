//! Call-media sender keys (plans C7.20, C7.29; RFC 9605 §5.1).
//!
//! Each participant of a community voice channel draws its own random
//! media secret and sends it to every other participant over a pairwise
//! channel; nobody else encrypts under it (RFC 9605 §4.4.1: "A given
//! base_key MUST NOT be used for encryption by multiple senders"). This is
//! how Signal group calls, MatrixRTC and Jitsi key media, and it is what
//! makes a split impossible: one writer per key.
//!
//! The pairwise channel is HPKE Base mode to the recipient's per-session
//! key, signed by the sender's pseudonym ([`session`]). The AAD binds the
//! channel, the recipient and the key index, so a sealed key cannot be
//! replayed into another channel, to another member, or as another index.

use hkdf::Hkdf;
use rand::RngCore;
use rekindle_types::domains;
use sha2::Sha512;
use zeroize::Zeroizing;

pub mod keyring;
pub mod session;

/// A fresh random media secret.
#[must_use]
pub fn fresh() -> Zeroizing<[u8; 32]> {
    let mut secret = Zeroizing::new([0u8; 32]);
    rand::rngs::OsRng.fill_bytes(secret.as_mut());
    secret
}

/// A random starting key generation for a channel session: 32 random
/// bits, so a rejoining participant's new indices never coincide with the
/// ones a receiver still holds from its previous session, and counting up
/// from it never wraps (`keyring`: index = generation ‖ 8-bit step).
#[must_use]
pub fn random_session_generation() -> u64 {
    u64::from(rand::rngs::OsRng.next_u32())
}

/// The AAD a sealed media key is bound to.
#[must_use]
pub fn seal_aad(channel_id: &str, recipient_hex: &str, key_index: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(channel_id.len() + recipient_hex.len() + 10);
    aad.extend_from_slice(channel_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(recipient_hex.as_bytes());
    aad.push(0);
    aad.extend_from_slice(&key_index.to_be_bytes());
    aad
}

/// A video sender's frame key: `HKDF-SHA512(salt = "", ikm = secret,
/// info = VIDEO_SENDER_KEY ‖ sender_key)`. Video seals whole frames with
/// its own AEAD, so it never uses the media secret itself, which SFrame
/// keys voice from.
#[must_use]
pub fn video_frame_key(secret: &[u8; 32], sender_key: &[u8]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha512>::new(Some(&[]), secret);
    let mut info = Vec::with_capacity(domains::VIDEO_SENDER_KEY.len() + sender_key.len());
    info.extend_from_slice(domains::VIDEO_SENDER_KEY.as_bytes());
    info.extend_from_slice(sender_key);
    let mut out = Zeroizing::new([0u8; 32]);
    let _ = hk.expand(&info, out.as_mut());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_frame_keys_differ_per_sender_and_from_the_secret() {
        let secret = fresh();
        let a = video_frame_key(&secret, b"alice");
        let b = video_frame_key(&secret, b"bob");
        assert_ne!(*a, *b);
        assert_ne!(*a, *secret);
    }
}
