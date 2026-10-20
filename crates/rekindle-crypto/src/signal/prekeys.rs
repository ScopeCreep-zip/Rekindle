//! The initiator's PQXDH input (PQXDH rev. 3 §3.2): a peer's prekeys as
//! [`crate::signal::SessionRecord::initiate`] takes them.
//!
//! Since plan E2.4 nothing carries a bundle on the wire: the initiator
//! reads the peer's prekey record (`PrekeyRecordHead` and one `OpkSlot`,
//! `schemas/identity.capnp`) and assembles the bundle with
//! [`PreKeyBundle::from_record`], which checks every signature under the
//! peer's **pinned** identity first. [`crate::signal::pqxdh::agreement`]
//! checks them again before any DH runs, so a bundle built any other way
//! is held to the same rules.

use ed25519_dalek::VerifyingKey;
use rekindle_codec::capnp_codec::identity::{OpkSlot, PrekeyRecordHead};

use crate::error::CryptoError;
use crate::signal::pqxdh::verify::{verify_opk, verify_pq, verify_spk, PqRole};
use crate::signal::pqxdh::PqxdhError;
use crate::signal::prekey_record::{head_is_current, is_prekey_id};

/// A peer device's public prekeys: the signed prekey and the ML-KEM
/// last-resort key, and optionally one slot's one-time keys.
#[derive(Debug, Clone)]
pub struct PreKeyBundle {
    // ── Classical X3DH layer ────────────────────────────────────
    /// Ed25519 identity public key.
    pub identity_key: Vec<u8>,
    /// The signed prekey's id, which the PreKeyInit names.
    pub signed_prekey_id: u32,
    /// X25519 signed prekey (public).
    pub signed_prekey: Vec<u8>,
    /// Ed25519 over `PREKEY_SPK_SIG ‖ u32be(id) ‖ i64be(issued_at_ms) ‖
    /// EncodeEC(signed_prekey)`.
    pub signed_prekey_signature: Vec<u8>,
    /// When the head (the signed prekey and the last-resort key) was
    /// minted, in ms; both of its signatures cover it.
    pub issued_at_ms: i64,
    /// Optional X25519 one-time prekey (consumed on first use).
    pub one_time_prekey: Option<Vec<u8>>,
    /// Identifier for the one-time X25519 prekey (matched on use).
    pub one_time_prekey_id: Option<u32>,
    /// Ed25519 over `PREKEY_OPK_SIG ‖ u32be(id) ‖ EncodeEC(one_time_prekey)`.
    pub one_time_prekey_signature: Option<Vec<u8>>,

    // ── PQXDH layer ─────────────────────────────────────────────
    /// The last-resort key's id, which the PreKeyInit names when `CT` is
    /// for it.
    pub pq_lr_id: u32,
    /// ML-KEM-768 last-resort public key (1184 bytes).
    pub pqpk_lr: Vec<u8>,
    /// Ed25519 over `PREKEY_PQ_LR_SIG ‖ u32be(id) ‖ i64be(issued_at_ms) ‖
    /// EncodeKEM(pqpk_lr)`.
    pub pqpk_lr_signature: Vec<u8>,
    /// Optional one-time ML-KEM-768 public key (1184 bytes).
    pub pqpk_ot: Option<Vec<u8>>,
    /// Ed25519 over `PREKEY_PQ_OT_SIG ‖ u32be(id) ‖ EncodeKEM(pqpk_ot)`.
    pub pqpk_ot_signature: Option<Vec<u8>>,
    /// Identifier for the one-time PQ prekey (matched on use).
    pub pqpk_ot_id: Option<u32>,
}

impl PreKeyBundle {
    /// The bundle from a peer's prekey record: its head and, unless the
    /// pick found none (PQXDH §2.5, §3.3), one slot. Every id must be a
    /// non-zero u31, every signature must verify under `identity_key`,
    /// the identity we pinned for the peer, not one the record claims, and
    /// the head's signed `issuedAt` must be within
    /// [`SIGNED_MAX_AGE_MS`](crate::signal::prekey_record::SIGNED_MAX_AGE_MS)
    /// of `now_ms`.
    ///
    /// # Errors
    /// An id out of range, a key of the wrong shape, or a signature that
    /// does not verify; [`CryptoError::StalePrekeys`] for a head that
    /// verifies but is out of date.
    pub fn from_record(
        identity_key: &[u8; 32],
        head: &PrekeyRecordHead,
        slot: Option<&OpkSlot>,
        now_ms: i64,
    ) -> Result<Self, CryptoError> {
        let ik = VerifyingKey::from_bytes(identity_key)
            .map_err(|e| CryptoError::invalid_key(format!("pinned identity: {e}")))?;
        let mut ids = vec![head.spk_id, head.pq_lr_id];
        if let Some(slot) = slot {
            ids.extend([slot.opk_id, slot.pq_opk_id]);
        }
        if let Some(id) = ids.into_iter().find(|id| !is_prekey_id(*id)) {
            return Err(CryptoError::invalid_key(format!(
                "prekey id {id} is not a non-zero u31"
            )));
        }
        verify_record(&ik, head, slot)
            .map_err(|e| CryptoError::invalid_key(format!("prekey record: {e}")))?;
        if !head_is_current(head.issued_at, now_ms) {
            return Err(CryptoError::StalePrekeys {
                issued_at_ms: head.issued_at,
                now_ms,
            });
        }
        Ok(Self {
            identity_key: identity_key.to_vec(),
            signed_prekey_id: head.spk_id,
            signed_prekey: head.spk.to_vec(),
            signed_prekey_signature: head.spk_sig.to_vec(),
            issued_at_ms: head.issued_at,
            one_time_prekey: slot.map(|s| s.opk.to_vec()),
            one_time_prekey_id: slot.map(|s| s.opk_id),
            one_time_prekey_signature: slot.map(|s| s.opk_sig.to_vec()),
            pq_lr_id: head.pq_lr_id,
            pqpk_lr: head.pq_lr.clone(),
            pqpk_lr_signature: head.pq_lr_sig.to_vec(),
            pqpk_ot: slot.map(|s| s.pq_opk.clone()),
            pqpk_ot_signature: slot.map(|s| s.pq_opk_sig.to_vec()),
            pqpk_ot_id: slot.map(|s| s.pq_opk_id),
        })
    }
}

/// Every signature of `head` and `slot` under `ik`.
fn verify_record(
    ik: &VerifyingKey,
    head: &PrekeyRecordHead,
    slot: Option<&OpkSlot>,
) -> Result<(), PqxdhError> {
    verify_spk(ik, head.spk_id, head.issued_at, &head.spk, &head.spk_sig)?;
    verify_pq(
        ik,
        PqRole::LastResort {
            issued_at_ms: head.issued_at,
        },
        head.pq_lr_id,
        &head.pq_lr,
        &head.pq_lr_sig,
    )?;
    if let Some(slot) = slot {
        verify_opk(ik, slot.opk_id, &slot.opk, &slot.opk_sig)?;
        verify_pq(
            ik,
            PqRole::OneTime,
            slot.pq_opk_id,
            &slot.pq_opk,
            &slot.pq_opk_sig,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::prekey_record::{mint_head, mint_slot};
    use crate::signal::test_support::TestRandom;
    use crate::Identity;

    #[test]
    fn a_record_assembles_a_bundle_with_and_without_a_slot() {
        let bob = Identity::generate();
        let rng = TestRandom::new(1);
        let head = mint_head(&bob, &rng, 11, 12, 1_000);
        let slot = mint_slot(&bob, &rng, 1, 21, 22);
        let bundle = PreKeyBundle::from_record(
            &bob.public_key_bytes(),
            &head.record,
            Some(&slot.record),
            1_000,
        )
        .unwrap();
        assert_eq!((bundle.signed_prekey_id, bundle.pq_lr_id), (11, 12));
        assert_eq!(
            (bundle.one_time_prekey_id, bundle.pqpk_ot_id),
            (Some(21), Some(22))
        );
        let bare =
            PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, None, 1_000).unwrap();
        assert!(bare.one_time_prekey.is_none() && bare.pqpk_ot.is_none());
    }

    /// The signatures are checked under the pinned identity: a record
    /// signed by anyone else is refused, as is a key moved to another id.
    #[test]
    fn a_record_under_another_identity_or_other_ids_is_refused() {
        let (bob, mallory) = (Identity::generate(), Identity::generate());
        let rng = TestRandom::new(2);
        let head = mint_head(&mallory, &rng, 11, 12, 1_000);
        assert!(
            PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, None, 1_000).is_err()
        );

        let head = mint_head(&bob, &rng, 11, 12, 1_000);
        let mut slot = mint_slot(&bob, &rng, 1, 21, 22).record;
        slot.opk_id = 23;
        assert!(PreKeyBundle::from_record(
            &bob.public_key_bytes(),
            &head.record,
            Some(&slot),
            1_000
        )
        .is_err());
        let mut moved = head.record.clone();
        moved.spk_id = 13;
        assert!(PreKeyBundle::from_record(&bob.public_key_bytes(), &moved, None, 1_000).is_err());
        let mut redated = head.record.clone();
        redated.issued_at += 1;
        assert!(PreKeyBundle::from_record(&bob.public_key_bytes(), &redated, None, 1_000).is_err());
    }

    #[test]
    fn an_id_outside_u31_is_refused() {
        let bob = Identity::generate();
        let rng = TestRandom::new(3);
        for id in [0, 0x8000_0000] {
            let head = mint_head(&bob, &rng, id, 12, 1_000);
            assert!(
                PreKeyBundle::from_record(&bob.public_key_bytes(), &head.record, None, 1_000)
                    .is_err(),
                "id {id}"
            );
        }
    }

    /// A head dated more than `SIGNED_MAX_AGE_MS` from now, either way, is
    /// refused as stale; one at the bound is used.
    #[test]
    fn a_head_outside_the_age_bound_is_refused() {
        use crate::signal::prekey_record::SIGNED_MAX_AGE_MS;
        let bob = Identity::generate();
        let rng = TestRandom::new(4);
        let at = 1_000_000_000_000;
        let head = mint_head(&bob, &rng, 11, 12, at).record;
        let read = |now: i64| PreKeyBundle::from_record(&bob.public_key_bytes(), &head, None, now);
        assert!(read(at + SIGNED_MAX_AGE_MS).is_ok());
        assert!(read(at - SIGNED_MAX_AGE_MS).is_ok());
        for now in [at + SIGNED_MAX_AGE_MS + 1, at - SIGNED_MAX_AGE_MS - 1] {
            assert!(matches!(read(now), Err(CryptoError::StalePrekeys { .. })));
        }
    }
}
