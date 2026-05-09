//! X25519 ECDH for DH ratchet steps.
//!
//! Two key types:
//! - `EphemeralPrivateKey`: single-use, consumed on agree. For PQXDH handshake.
//! - `PrivateKey`: reusable, borrowed on agree. For ratchet DH keys that persist
//!   across multiple messages until the next DH ratchet step.
//!
//! `PrivateKey::from_private_key(&X25519, &seed)` reconstructs from stored seed.
//! `AsBigEndian<Curve25519SeedBin>::as_be_bytes()` extracts the seed for persistence.
//!
//! # Zeroization
//!
//! `PrivateKey` wraps `LcPtr<EVP_PKEY>` (C-allocated). aws-lc-rs 1.16 does NOT
//! zeroize the scalar on drop — `EVP_PKEY_free` frees without zeroing. The scalar
//! persists in freed C heap until overwritten by a future allocation.
//!
//! Mitigation: we extract the seed into `Zeroizing<[u8; 32]>` immediately after
//! generation and store the seed (zeroized on drop). `PrivateKey` is reconstructed
//! from the seed on each use and dropped immediately after DH agree. The C-heap
//! exposure window is minimized to the duration of a single `ratchet_agree` call.
//!
//! The authoritative copy of the private key is the `Zeroizing<[u8; 32]>` seed
//! stored in `DoubleRatchetState.dhs_priv`, NOT the `PrivateKey` object.

use aws_lc_rs::agreement::{self, UnparsedPublicKey, X25519};
use aws_lc_rs::encoding::AsBigEndian;
use aws_lc_rs::rand;
use zeroize::Zeroizing;

use crate::error::RatchetError;

/// Generate a fresh X25519 ephemeral keypair (single-use, for PQXDH).
///
/// The private key is consumed by [`ephemeral_agree`].
pub fn generate_ephemeral() -> Result<(agreement::EphemeralPrivateKey, [u8; 32]), RatchetError> {
    let rng = rand::SystemRandom::new();
    let sk = agreement::EphemeralPrivateKey::generate(&X25519, &rng)
        .map_err(|_| RatchetError::Ecdh)?;
    let pk = sk.compute_public_key().map_err(|_| RatchetError::Ecdh)?;
    let mut pk_bytes = [0u8; 32];
    pk_bytes.copy_from_slice(pk.as_ref());
    Ok((sk, pk_bytes))
}

/// Single-use ECDH agreement (consumes the private key). For PQXDH.
pub fn ephemeral_agree(
    our_private: agreement::EphemeralPrivateKey,
    their_public: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    let peer = UnparsedPublicKey::new(&X25519, their_public);
    let mut out = Zeroizing::new([0u8; 32]);
    agreement::agree_ephemeral(our_private, peer, RatchetError::Ecdh, |km| {
        if km.len() != 32 {
            return Err(RatchetError::Ecdh);
        }
        out.copy_from_slice(km);
        Ok(())
    })?;
    Ok(out)
}

/// Generate a reusable X25519 keypair (for DH ratchet steps).
///
/// Returns `(seed, public_key)`. The seed must be stored for persistence
/// and passed to [`reusable_from_seed`] on session restore.
pub fn generate_ratchet_keypair() -> Result<(Zeroizing<[u8; 32]>, [u8; 32]), RatchetError> {
    let sk = agreement::PrivateKey::generate(&X25519).map_err(|_| RatchetError::Ecdh)?;
    let pk = sk.compute_public_key().map_err(|_| RatchetError::Ecdh)?;
    let mut pk_bytes = [0u8; 32];
    pk_bytes.copy_from_slice(pk.as_ref());

    // Extract seed for persistence
    let seed_bin: aws_lc_rs::encoding::Curve25519SeedBin<'_> =
        sk.as_be_bytes().map_err(|_| RatchetError::Ecdh)?;
    let seed_ref = seed_bin.as_ref();
    if seed_ref.len() != 32 {
        return Err(RatchetError::Ecdh);
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    seed.copy_from_slice(seed_ref);

    Ok((seed, pk_bytes))
}

/// Reconstruct a reusable X25519 private key from a stored seed.
pub fn reusable_from_seed(seed: &[u8; 32]) -> Result<agreement::PrivateKey, RatchetError> {
    agreement::PrivateKey::from_private_key(&X25519, seed)
        .map_err(|_| RatchetError::Ecdh)
}

/// Reusable ECDH agreement (borrows the private key). For DH ratchet steps.
///
/// The private key can be used for multiple agreements — once per inbound
/// message until the next DH ratchet step.
pub fn ratchet_agree(
    our_private: &agreement::PrivateKey,
    their_public: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    let peer = UnparsedPublicKey::new(&X25519, their_public);
    let mut out = Zeroizing::new([0u8; 32]);
    agreement::agree(our_private, peer, RatchetError::Ecdh, |km| {
        if km.len() != 32 {
            return Err(RatchetError::Ecdh);
        }
        out.copy_from_slice(km);
        Ok(())
    })?;
    Ok(out)
}
