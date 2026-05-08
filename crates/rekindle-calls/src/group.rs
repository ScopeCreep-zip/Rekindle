//! Group call key wrap (Wave 12 W12.9 — architecture §10.10).
//!
//! Group calls reuse the existing 1:1 voice / video transport, but a
//! single shared `call_key` is needed across all participants. The
//! initiator generates the key once, then for each invitee derives a
//! per-recipient wrap key via X25519 ECDH + HKDF-SHA256 and seals the
//! call_key with AES-256-GCM. Each invitee's `GroupCallOffer` envelope
//! carries only their own wrap; another invitee can't unwrap a
//! recipient's blob because they don't hold that recipient's X25519
//! private key.
//!
//! Wire format for `wrapped_call_key`:
//!
//! ```text
//!   12B  AES-GCM nonce  (deterministic from blake3(call_id || recipient_pubkey)[..12])
//!   32B  ciphertext     (the call_key sealed under wrap_key)
//!   16B  authentication tag (appended by aes-gcm crate)
//! ```
//!
//! Total = 60 bytes per recipient, well under the 32 KB app_call cap.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

/// Domain-separated info string. Kept distinct from the 1:1
/// `derive_call_key` path so the same X25519 keypair across both
/// contexts can never produce the same wrap_key.
const HKDF_INFO: &[u8] = b"rekindle-group-call-wrap-v1";

#[derive(Debug, Error)]
pub enum GroupKeyError {
    #[error("peer x25519 public key must be 32 bytes, got {0}")]
    InvalidPublicKey(usize),
    #[error("hkdf expand failed: {0}")]
    Hkdf(String),
    #[error("aes-gcm seal/open failed (likely wrong recipient): {0}")]
    Aead(String),
    #[error("wrapped key payload must be at least 28 bytes, got {0}")]
    Truncated(usize),
}

/// Generate a fresh 32-byte call_key. The initiator creates this once
/// per group call and never derives it from anyone's identity — each
/// participant only sees the sealed wrap from `wrap_call_key`.
#[must_use]
pub fn generate_call_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Derive a deterministic 12-byte AES-GCM nonce from
/// `(call_id, recipient_pubkey)` so the same wrap is always associated
/// with the same nonce — defense against accidental nonce reuse if the
/// initiator decides to re-issue an offer to a participant.
fn nonce_for(call_id: &str, recipient_pubkey: &str) -> [u8; 12] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(call_id.as_bytes());
    hasher.update(b"\x00");
    hasher.update(recipient_pubkey.as_bytes());
    let h = hasher.finalize();
    let mut out = [0u8; 12];
    out.copy_from_slice(&h.as_bytes()[..12]);
    out
}

fn derive_wrap_key(
    initiator_secret: &StaticSecret,
    recipient_x25519_pub: &[u8; 32],
) -> Result<[u8; 32], GroupKeyError> {
    let peer_pub = PublicKey::from(*recipient_x25519_pub);
    let shared = initiator_secret.diffie_hellman(&peer_pub);
    let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut wrap_key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut wrap_key)
        .map_err(|e| GroupKeyError::Hkdf(e.to_string()))?;
    Ok(wrap_key)
}

/// Wrap the shared `call_key` so only the recipient (who holds the
/// matching X25519 private) can unseal it.
pub fn wrap_call_key(
    initiator_secret: &StaticSecret,
    recipient_x25519_pub: &[u8],
    call_id: &str,
    recipient_pubkey: &str,
    call_key: &[u8; 32],
) -> Result<Vec<u8>, GroupKeyError> {
    if recipient_x25519_pub.len() != 32 {
        return Err(GroupKeyError::InvalidPublicKey(recipient_x25519_pub.len()));
    }
    let mut peer_arr = [0u8; 32];
    peer_arr.copy_from_slice(recipient_x25519_pub);

    let wrap_key = derive_wrap_key(initiator_secret, &peer_arr)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&wrap_key));
    let nonce = nonce_for(call_id, recipient_pubkey);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), call_key.as_slice())
        .map_err(|e| GroupKeyError::Aead(e.to_string()))?;

    // Layout: nonce || ciphertext (which already includes the 16-byte tag).
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Unseal the call_key sent in our `GroupCallOffer`. We're the
/// recipient; `our_secret` is our X25519 private; `initiator_x25519_pub`
/// came on the offer envelope.
pub fn unwrap_call_key(
    our_secret: &StaticSecret,
    initiator_x25519_pub: &[u8],
    call_id: &str,
    our_pubkey: &str,
    wrapped: &[u8],
) -> Result<[u8; 32], GroupKeyError> {
    if initiator_x25519_pub.len() != 32 {
        return Err(GroupKeyError::InvalidPublicKey(initiator_x25519_pub.len()));
    }
    if wrapped.len() < 28 {
        return Err(GroupKeyError::Truncated(wrapped.len()));
    }
    let mut peer_arr = [0u8; 32];
    peer_arr.copy_from_slice(initiator_x25519_pub);

    let wrap_key = derive_wrap_key(our_secret, &peer_arr)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&wrap_key));

    let (nonce_bytes, ciphertext) = wrapped.split_at(12);
    let expected_nonce = nonce_for(call_id, our_pubkey);
    if nonce_bytes != expected_nonce {
        return Err(GroupKeyError::Aead(
            "nonce mismatch (wrong recipient or tampered)".into(),
        ));
    }
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|e| GroupKeyError::Aead(e.to_string()))?;
    if plaintext.len() != 32 {
        return Err(GroupKeyError::Aead(format!(
            "unwrapped call_key has unexpected length {}",
            plaintext.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&plaintext);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fresh_keypair;

    #[test]
    fn round_trip_wraps_for_each_participant() {
        let (alice_sk, _alice_pub) = fresh_keypair();
        let (bob_sk, bob_pub) = fresh_keypair();
        let (carol_sk, carol_pub) = fresh_keypair();

        let call_key = generate_call_key();
        let call_id = "group-call-xyz";

        let bob_wrap = wrap_call_key(&alice_sk, &bob_pub, call_id, "bob_pub", &call_key).unwrap();
        let carol_wrap =
            wrap_call_key(&alice_sk, &carol_pub, call_id, "carol_pub", &call_key).unwrap();

        let alice_x25519_pub = x25519_dalek::PublicKey::from(&alice_sk).to_bytes();
        let bob_unwrapped =
            unwrap_call_key(&bob_sk, &alice_x25519_pub, call_id, "bob_pub", &bob_wrap).unwrap();
        let carol_unwrapped = unwrap_call_key(
            &carol_sk,
            &alice_x25519_pub,
            call_id,
            "carol_pub",
            &carol_wrap,
        )
        .unwrap();

        assert_eq!(bob_unwrapped, call_key);
        assert_eq!(carol_unwrapped, call_key);
    }

    #[test]
    fn other_participant_cannot_unwrap() {
        let (alice_sk, _) = fresh_keypair();
        let (bob_sk, bob_pub) = fresh_keypair();
        let (carol_sk, _) = fresh_keypair();

        let call_key = generate_call_key();
        let call_id = "exclusive";
        let bob_wrap = wrap_call_key(&alice_sk, &bob_pub, call_id, "bob", &call_key).unwrap();

        let alice_x25519_pub = x25519_dalek::PublicKey::from(&alice_sk).to_bytes();
        // Carol cannot decrypt Bob's wrap because she doesn't hold Bob's secret.
        let result = unwrap_call_key(&carol_sk, &alice_x25519_pub, call_id, "bob", &bob_wrap);
        assert!(result.is_err());
        // Bob using Carol's pubkey label also fails (nonce mismatch).
        let result = unwrap_call_key(&bob_sk, &alice_x25519_pub, call_id, "carol", &bob_wrap);
        assert!(result.is_err());
    }

    #[test]
    fn truncated_wrap_rejected() {
        let (alice_sk, _) = fresh_keypair();
        let alice_pub = x25519_dalek::PublicKey::from(&alice_sk).to_bytes();
        let result = unwrap_call_key(&alice_sk, &alice_pub, "x", "y", &[0u8; 10]);
        assert!(matches!(result, Err(GroupKeyError::Truncated(10))));
    }
}
