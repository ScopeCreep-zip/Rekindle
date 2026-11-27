//! MEK (Media Encryption Key) distribution via X25519 ECDH + HKDF + AES-256-GCM.
//!
//! Thin façade over `rekindle_secrets::mek` — the single wrap/unwrap
//! implementation shared by every track (same HKDF info label
//! `rekindle-mek-wrap-v1`, same `[12-byte nonce || ciphertext+tag]` wire
//! format, and a zeroizing wrapping key per audit finding P7-W26). This
//! module used to carry its own copy of the algorithm, wire-compatible by
//! hand with the secrets and transport copies; only the
//! `rekindle_types::error::CryptoError → crate::error::CryptoError`
//! mapping lives here now.
//!
//! Wire format: `[12-byte nonce || ciphertext + 16-byte tag]`
//! (68 bytes for the 40-byte MEK wire input: 8-byte generation LE + 32-byte key).

use ed25519_dalek::SigningKey;

use crate::error::CryptoError;

/// Wrap (encrypt) MEK wire bytes for a specific recipient.
///
/// - `sender_signing_key`: The wrapping peer's pseudonym Ed25519 signing key.
/// - `recipient_ed25519_public`: The target member's pseudonym Ed25519 public key bytes.
/// - `mek_wire_bytes`: The 40-byte MEK wire format (generation LE + key material).
///
/// Returns: `[12-byte nonce || ciphertext+tag]` (68 bytes for a 40-byte input).
pub fn wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    rekindle_secrets::mek::wrap_mek(sender_signing_key, recipient_ed25519_public, mek_wire_bytes)
        .map_err(Into::into)
}

/// Wrap MEK wire bytes via RFC 9180 HPKE (v2 format, `0x02`-prefixed).
///
/// Senders flip from [`wrap_mek`] to this once every deployed reader
/// carries the dual-read [`unwrap_mek`] — see
/// `rekindle_secrets::mek` module docs for the rollout contract.
pub fn hpke_wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    rekindle_secrets::mek::hpke_wrap_mek(
        sender_signing_key,
        recipient_ed25519_public,
        mek_wire_bytes,
    )
    .map_err(Into::into)
}

/// Unwrap (decrypt) MEK wire bytes received from a peer. Reads BOTH
/// wire formats — legacy `[nonce || ct+tag]` and HPKE
/// `[0x02 || enc || ct+tag]` (see `rekindle_secrets::mek`).
///
/// - `recipient_signing_key`: Our pseudonym Ed25519 signing key.
/// - `sender_ed25519_public`: The wrapping peer's pseudonym Ed25519 public key bytes.
///
/// Returns: The decrypted MEK wire bytes (40 bytes).
pub fn unwrap_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    wrapped_mek: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    rekindle_secrets::mek::unwrap_mek(recipient_signing_key, sender_ed25519_public, wrapped_mek)
        .map_err(Into::into)
}

#[cfg(test)]
#[path = "mek_distribution_tests.rs"]
mod tests;
