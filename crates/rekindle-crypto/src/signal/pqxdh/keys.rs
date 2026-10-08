//! Conversion helpers between wire-format bytes and key types.

use ed25519_dalek::VerifyingKey;
use rekindle_secrets::pq_keys::MlKemPublic;
use x25519_dalek::PublicKey as X25519Public;

use super::PqxdhError;

/// Parse a 32-byte X25519 public key from wire bytes.
pub fn x25519_from_bytes(bytes: &[u8]) -> Result<X25519Public, PqxdhError> {
    if bytes.len() != 32 {
        return Err(PqxdhError::InvalidX25519Length(bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    Ok(X25519Public::from(arr))
}

/// Convert an Ed25519 verifying key into an X25519 public key (Montgomery
/// form). Used to derive an X25519 DH partner from the peer's Ed25519
/// identity key — matches the X3DH convention.
///
/// `ed25519-dalek` exposes `VerifyingKey::to_montgomery()` returning a
/// `MontgomeryPoint`. X25519's `PublicKey` is the same byte representation.
/// Infallible; returns `X25519Public` directly.
#[must_use]
pub fn x25519_from_ed(ed: &VerifyingKey) -> X25519Public {
    X25519Public::from(ed.to_montgomery().to_bytes())
}

/// Parse an ML-KEM-768 public key from its 1184-byte wire representation.
pub fn ml_kem_public_from_bytes(bytes: &[u8]) -> Result<MlKemPublic, PqxdhError> {
    MlKemPublic::from_bytes(bytes).ok_or(PqxdhError::InvalidMlKemPublic)
}
