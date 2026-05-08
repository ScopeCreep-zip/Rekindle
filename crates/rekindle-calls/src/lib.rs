//! Direct call key derivation and state.
//!
//! Architecture §10.10 (Chiralgrams) Tier 7 — direct calls between two
//! peers derive a 32-byte `call_key` via X25519 ECDH plus HKDF-SHA256.
//! The shared secret is used by `rekindle-voice` to encrypt audio /
//! video frames over the `app_message` transport.

#![forbid(unsafe_code)]

pub mod group;
pub mod state;

use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

pub use state::{CallKind, CallState, CallStatus};

// Wave 12 W12.9 — re-export the X25519 types so consumer crates
// (src-tauri/services/group_calls.rs) don't have to depend on
// x25519-dalek directly.
pub use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519StaticSecret};

/// HKDF info string. Distinct from the friend / DM key derivations so
/// the same X25519 keypair can never produce the same secret across
/// contexts (domain separation per RFC 5869 §3.2).
const HKDF_INFO: &[u8] = b"rekindle-call-key-v1";

#[derive(Debug, Error)]
pub enum CallKeyError {
    #[error("peer x25519 public key must be 32 bytes, got {0}")]
    InvalidPublicKey(usize),
    #[error("hkdf expand failed: {0}")]
    Hkdf(String),
}

/// Derive a 32-byte symmetric key shared between caller and callee.
///
/// Inputs:
/// * `my_x25519_secret` — the local ephemeral (or long-lived) X25519
///   secret. Pass [`StaticSecret`] so it's zeroized on drop.
/// * `peer_x25519_pub` — the peer's X25519 public key (32 bytes).
/// * `call_id` — the call identifier negotiated in `CallOffer`. Used
///   as HKDF salt so two simultaneous calls between the same pair
///   produce different `call_key`s.
///
/// Output is the same on both sides because X25519 ECDH is symmetric:
/// `aliceSecret * bobPub == bobSecret * alicePub`.
pub fn derive_call_key(
    my_x25519_secret: &StaticSecret,
    peer_x25519_pub: &[u8],
    call_id: &str,
) -> Result<[u8; 32], CallKeyError> {
    if peer_x25519_pub.len() != 32 {
        return Err(CallKeyError::InvalidPublicKey(peer_x25519_pub.len()));
    }
    let mut peer_arr = [0u8; 32];
    peer_arr.copy_from_slice(peer_x25519_pub);
    let peer_pub = PublicKey::from(peer_arr);
    let shared = my_x25519_secret.diffie_hellman(&peer_pub);

    let hk = Hkdf::<Sha256>::new(Some(call_id.as_bytes()), shared.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(HKDF_INFO, &mut out)
        .map_err(|e| CallKeyError::Hkdf(e.to_string()))?;
    Ok(out)
}

/// Generate a fresh ephemeral X25519 keypair for a new call. The
/// secret is wrapped in [`StaticSecret`] so it zeroes on drop; we use
/// `StaticSecret` rather than `EphemeralSecret` because the caller
/// needs to keep it across the request/reply round-trip.
#[must_use]
pub fn fresh_keypair() -> (StaticSecret, [u8; 32]) {
    let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
    let public = PublicKey::from(&secret);
    (secret, public.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_key_matches_on_both_sides() {
        let (alice_sk, alice_pub) = fresh_keypair();
        let (bob_sk, bob_pub) = fresh_keypair();

        let alice_key = derive_call_key(&alice_sk, &bob_pub, "call-abc").unwrap();
        let bob_key = derive_call_key(&bob_sk, &alice_pub, "call-abc").unwrap();
        assert_eq!(alice_key, bob_key);
    }

    #[test]
    fn different_call_ids_produce_different_keys() {
        let (alice_sk, _) = fresh_keypair();
        let (_, bob_pub) = fresh_keypair();

        let k1 = derive_call_key(&alice_sk, &bob_pub, "call-1").unwrap();
        let k2 = derive_call_key(&alice_sk, &bob_pub, "call-2").unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn rejects_wrong_length_pubkey() {
        let (alice_sk, _) = fresh_keypair();
        let result = derive_call_key(&alice_sk, &[0u8; 16], "x");
        assert!(matches!(result, Err(CallKeyError::InvalidPublicKey(16))));
    }
}
