//! `PreKey` bundle wire type for Signal Protocol session establishment.
//!
//! Bundles are built by [`crate::signal::session::SignalSessionManager`]:
//! - `current_bundle` — the long-lived signed prekey and PQ last-resort
//!   key, with no one-time keys; published to DHT profile subkey 5 and
//!   carried in invites.
//! - `handout_bundle` — the current bundle plus a fresh one-time X25519
//!   and ML-KEM-768 key, for a bundle sent to exactly one peer.
//!
//! Phase 3b of the decomposed-harvest plan augmented this struct with
//! PQXDH ML-KEM-768 fields. Wire format breaks for any peer expecting
//! the classical-only shape — pre-ship hard break per invariant #5.

use serde::{Deserialize, Serialize};

use crate::error::CryptoError;

/// A bundle of public keys published to DHT for session establishment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreKeyBundle {
    // ── Classical X3DH layer ────────────────────────────────────
    /// Ed25519 identity public key.
    pub identity_key: Vec<u8>,
    /// X25519 signed prekey (public).
    pub signed_prekey: Vec<u8>,
    /// Ed25519 signature over `0x01 || signed_prekey`.
    pub signed_prekey_signature: Vec<u8>,
    /// Optional X25519 one-time prekey (consumed on first use).
    pub one_time_prekey: Option<Vec<u8>>,
    /// Identifier for the one-time X25519 prekey (matched on use).
    pub one_time_prekey_id: Option<u32>,
    /// Registration ID for Signal Protocol.
    pub registration_id: u32,

    // ── PQXDH layer ─────────────────────────────────────────────
    /// ML-KEM-768 last-resort public key (1184 bytes).
    pub pqpk_lr: Vec<u8>,
    /// Ed25519 signature over `0x02 || "LR" || pqpk_lr`.
    pub pqpk_lr_signature: Vec<u8>,
    /// Optional one-time ML-KEM-768 public key (1184 bytes).
    pub pqpk_ot: Option<Vec<u8>>,
    /// Ed25519 signature over `0x02 || "OT" || pqpk_ot`.
    pub pqpk_ot_signature: Option<Vec<u8>>,
    /// Identifier for the one-time PQ prekey (matched on use).
    pub pqpk_ot_id: Option<u32>,
}

impl PreKeyBundle {
    /// The bundle's one wire form, on both tracks and everywhere it
    /// travels (profile subkey 5, FriendRequest, FriendAccept, session
    /// reset): the packed Cap'n Proto `PreKeyBundle` of
    /// `schemas/identity.capnp`. It was JSON on the desktop and postcard
    /// on the daemon, so neither track read the other's, and the JSON form
    /// nested in a JSON payload put the friend handshake over Veilid's
    /// 32,768-byte message cap (plan C7.25).
    ///
    /// An absent one-time key is an empty field with id 0. Real one-time
    /// ids are never 0 (`session::bundles::fresh_id`), so `from_wire`
    /// reads 0 back as `None`.
    #[must_use]
    pub fn to_wire(&self) -> Vec<u8> {
        rekindle_codec::capnp_codec::identity::encode_prekey_bundle(
            &rekindle_codec::capnp_codec::identity::PreKeyBundle {
                identity_key: self.identity_key.clone(),
                signed_pre_key: self.signed_prekey.clone(),
                signed_pre_key_sig: self.signed_prekey_signature.clone(),
                one_time_pre_key: self.one_time_prekey.clone().unwrap_or_default(),
                one_time_pre_key_id: self.one_time_prekey_id.unwrap_or(0),
                registration_id: self.registration_id,
                pqpk_lr: self.pqpk_lr.clone(),
                pqpk_lr_sig: self.pqpk_lr_signature.clone(),
                pqpk_ot: self.pqpk_ot.clone().unwrap_or_default(),
                pqpk_ot_sig: self.pqpk_ot_signature.clone().unwrap_or_default(),
                pqpk_ot_id: self.pqpk_ot_id.unwrap_or(0),
            },
        )
    }

    /// Parse [`Self::to_wire`]'s form.
    pub fn from_wire(data: &[u8]) -> Result<Self, CryptoError> {
        let dto = rekindle_codec::capnp_codec::identity::decode_prekey_bundle(data)
            .map_err(|e| CryptoError::invalid_key(format!("prekey bundle: {e}")))?;
        let present = |bytes: Vec<u8>| (!bytes.is_empty()).then_some(bytes);
        let id = |id: u32| (id != 0).then_some(id);
        Ok(Self {
            identity_key: dto.identity_key,
            signed_prekey: dto.signed_pre_key,
            signed_prekey_signature: dto.signed_pre_key_sig,
            one_time_prekey: present(dto.one_time_pre_key),
            one_time_prekey_id: id(dto.one_time_pre_key_id),
            registration_id: dto.registration_id,
            pqpk_lr: dto.pqpk_lr,
            pqpk_lr_signature: dto.pqpk_lr_sig,
            pqpk_ot: present(dto.pqpk_ot),
            pqpk_ot_signature: present(dto.pqpk_ot_sig),
            pqpk_ot_id: id(dto.pqpk_ot_id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PreKeyBundle;

    fn bundle(one_time: bool) -> PreKeyBundle {
        PreKeyBundle {
            identity_key: vec![1; 32],
            signed_prekey: vec![2; 32],
            signed_prekey_signature: vec![3; 64],
            one_time_prekey: one_time.then(|| vec![4; 32]),
            one_time_prekey_id: one_time.then_some(7),
            registration_id: 9,
            pqpk_lr: vec![5; 1184],
            pqpk_lr_signature: vec![6; 64],
            pqpk_ot: one_time.then(|| vec![8; 1184]),
            pqpk_ot_signature: one_time.then(|| vec![10; 64]),
            pqpk_ot_id: one_time.then_some(11),
        }
    }

    #[test]
    fn the_wire_form_round_trips_with_and_without_one_time_keys() {
        for one_time in [true, false] {
            let original = bundle(one_time);
            let decoded = PreKeyBundle::from_wire(&original.to_wire()).unwrap();
            assert_eq!(decoded.one_time_prekey, original.one_time_prekey);
            assert_eq!(decoded.one_time_prekey_id, original.one_time_prekey_id);
            assert_eq!(decoded.pqpk_ot, original.pqpk_ot);
            assert_eq!(decoded.pqpk_ot_signature, original.pqpk_ot_signature);
            assert_eq!(decoded.pqpk_ot_id, original.pqpk_ot_id);
            assert_eq!(decoded.pqpk_lr, original.pqpk_lr);
            assert_eq!(decoded.identity_key, original.identity_key);
            assert_eq!(decoded.registration_id, original.registration_id);
        }
    }

    #[test]
    fn a_handout_bundle_is_binary_sized() {
        // Two ML-KEM-768 keys dominate: the wire form stays near the raw
        // key bytes, where the JSON form was ~3.6× them.
        assert!(bundle(true).to_wire().len() < 2_800);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(PreKeyBundle::from_wire(b"not a bundle").is_err());
    }
}
