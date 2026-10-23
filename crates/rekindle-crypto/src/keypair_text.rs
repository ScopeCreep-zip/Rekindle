//! A record writer's keypair as text, without Veilid (plan C9.1b): the
//! form Veilid prints and parses, `KIND:<public>:<secret>` with each key
//! base64url-nopad (`~/veilid` @0cd847de `crypto/types/keypair.rs`
//! `BareKeyPair::encode`, `crypto_typed.rs` `Display`). The record ports
//! carry keypairs this way, so the transport, the hosts and the netsim
//! never build a Veilid type; `rekindle-protocol` tests that Veilid reads
//! exactly this text.
//!
//! The 64-byte form (`public ‖ secret`) is wire-visible: a friend-inbox
//! keypair is published in the profile and a DM-log keypair travels in a
//! friend accept, so its bytes never change.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::SigningKey;

use crate::CryptoError;

/// The crypto kind every Rekindle record key and writer uses.
const KIND: &str = "VLD0";

/// The text of the keypair `(public, secret)`.
#[must_use]
pub fn keypair_text(public: &[u8; 32], secret: &[u8; 32]) -> String {
    format!(
        "{KIND}:{}:{}",
        URL_SAFE_NO_PAD.encode(public),
        URL_SAFE_NO_PAD.encode(secret)
    )
}

/// The text of a public key, as a record descriptor names its owner
/// (`RecordPort::owner_of`): `KIND:<public>`.
#[must_use]
pub fn public_key_text(public: &[u8; 32]) -> String {
    format!("{KIND}:{}", URL_SAFE_NO_PAD.encode(public))
}

/// The text of an Ed25519 signing key's keypair.
#[must_use]
pub fn keypair_text_from_signing(signing: &SigningKey) -> String {
    keypair_text(&signing.verifying_key().to_bytes(), &signing.to_bytes())
}

/// The `(public, secret)` halves of a keypair's text.
///
/// # Errors
/// The text is not a `VLD0` keypair of two 32-byte keys.
pub fn parse_keypair_text(text: &str) -> Result<([u8; 32], [u8; 32]), CryptoError> {
    let bad = |why: &str| CryptoError::invalid_key(format!("keypair text: {why}"));
    let mut parts = text.split(':');
    if parts.next() != Some(KIND) {
        return Err(bad("not a VLD0 keypair"));
    }
    let (Some(public), Some(secret), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(bad("expected KIND:public:secret"));
    };
    let decode = |part: &str| -> Result<[u8; 32], CryptoError> {
        URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|_| bad("not base64url"))?
            .try_into()
            .map_err(|_| bad("a key is not 32 bytes"))
    };
    Ok((decode(public)?, decode(secret)?))
}

/// The 64-byte wire form (`public ‖ secret`) of a keypair's text.
///
/// # Errors
/// As [`parse_keypair_text`].
pub fn keypair_wire(text: &str) -> Result<[u8; 64], CryptoError> {
    let (public, secret) = parse_keypair_text(text)?;
    let mut wire = [0u8; 64];
    wire[..32].copy_from_slice(&public);
    wire[32..].copy_from_slice(&secret);
    Ok(wire)
}

/// A keypair's text from its 64-byte wire form.
///
/// # Errors
/// `wire` is not 64 bytes.
pub fn keypair_text_from_wire(wire: &[u8]) -> Result<String, CryptoError> {
    let wire: &[u8; 64] = wire.try_into().map_err(|_| {
        CryptoError::invalid_key(format!("keypair wire form is {} bytes, not 64", wire.len()))
    })?;
    let (public, secret) = wire.split_at(32);
    let half = |h: &[u8]| -> [u8; 32] {
        let mut key = [0u8; 32];
        key.copy_from_slice(h);
        key
    };
    Ok(keypair_text(&half(public), &half(secret)))
}

/// The writer text of SMPL member slot `slot`, derived from the community's
/// slot seed (`rekindle_secrets::derive::derive_slot_keypair`).
///
/// # Errors
/// The derivation failed.
pub fn slot_keypair_text(seed: &[u8; 32], slot: u32) -> Result<String, CryptoError> {
    let signing = rekindle_secrets::derive::derive_slot_keypair(seed, slot)?;
    Ok(keypair_text_from_signing(&signing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_wire_round_trip() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let text = keypair_text_from_signing(&signing);
        assert!(text.starts_with("VLD0:"));
        let (public, secret) = parse_keypair_text(&text).unwrap();
        assert_eq!(public, signing.verifying_key().to_bytes());
        assert_eq!(secret, signing.to_bytes());
        let wire = keypair_wire(&text).unwrap();
        assert_eq!(keypair_text_from_wire(&wire).unwrap(), text);
    }

    #[test]
    fn malformed_text_is_refused() {
        assert!(parse_keypair_text("VLD1:AAAA:AAAA").is_err());
        assert!(parse_keypair_text("VLD0:AAAA").is_err());
        assert!(parse_keypair_text("VLD0:AAAA:AAAA:AAAA").is_err());
        assert!(parse_keypair_text("VLD0:!!:??").is_err());
        assert!(keypair_text_from_wire(&[0u8; 63]).is_err());
    }

    #[test]
    fn a_slot_keypair_is_the_seed_derivation() {
        let seed = [3u8; 32];
        let signing = rekindle_secrets::derive::derive_slot_keypair(&seed, 5).unwrap();
        assert_eq!(
            slot_keypair_text(&seed, 5).unwrap(),
            keypair_text_from_signing(&signing)
        );
    }
}
