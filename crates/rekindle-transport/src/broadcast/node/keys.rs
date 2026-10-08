//! Veilid keypair (de)serialization helpers.

use crate::error::{Result, TransportError};

/// Deserialize a keypair from the 64-byte format used by `serialize_keypair`.
///
/// Format: public key bytes (32) + secret key bytes (32).
pub fn deserialize_keypair(bytes: &[u8]) -> Result<veilid_core::KeyPair> {
    if bytes.len() != 64 {
        return Err(TransportError::Internal(format!(
            "keypair bytes: expected 64, got {}",
            bytes.len()
        )));
    }
    let bare_pub = veilid_core::BarePublicKey::new(&bytes[..32]);
    let bare_secret = veilid_core::BareSecretKey::new(&bytes[32..]);
    let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    Ok(veilid_core::KeyPair::new_from_parts(
        veilid_pub,
        bare_secret,
    ))
}

/// Serialize a Veilid `KeyPair` to bytes for keyring storage.
/// Format: public key bytes (32) + secret key bytes (32) = 64 bytes.
pub fn serialize_keypair(kp: &veilid_core::KeyPair) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(64);
    // veilid-core 0.5.3: `BareKey::bytes()` returns an owned `bytes::Bytes`
    // (was `&[u8]` in 0.5.2); `as_ref()` gives the `&[u8]` extend_from_slice wants.
    bytes.extend_from_slice(kp.key().value().bytes().as_ref());
    bytes.extend_from_slice(kp.secret().value().bytes().as_ref());
    bytes
}

/// Convert an Ed25519 signing key to a Veilid `KeyPair`.
pub fn ed25519_to_keypair(signing_key: &ed25519_dalek::SigningKey) -> veilid_core::KeyPair {
    let pub_bytes = signing_key.verifying_key().to_bytes();
    let secret_bytes = signing_key.to_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret)
}
