//! Minting and signing the record's values: a head, a slot, and the full
//! set (the head and every slot) a new identity publishes before its rows
//! exist (the identity ceremony, `rotate_identity`). Each value is signed
//! by the identity under `pqxdh::verify`'s payloads; Ed25519 is
//! deterministic, so re-signing stored public keys reproduces the bytes
//! that were published (the unlock reconciliation compares them).

use std::collections::BTreeSet;

use rekindle_codec::capnp_codec::identity::{OpkSlot, PrekeyRecordHead};
use rekindle_types::dht_layout::prekeys::{slot_subkey, SLOT_COUNT};
use rekindle_utils::random::RandomSource;

use super::random_id;
use crate::error::CryptoError;
use crate::signal::pqxdh::keys::{ml_kem_public_from_bytes, x25519_from_bytes};
use crate::signal::pqxdh::verify::{
    opk_signing_payload, pq_signing_payload, spk_signing_payload, PqRole,
};
use crate::signal::prekey_material::{mint_curve, mint_pq, MintedPrekey};
use crate::Identity;

/// A minted head: the secrets to store and the value to publish.
pub struct MintedHead {
    pub spk_id: u32,
    pub spk: MintedPrekey,
    pub pq_lr_id: u32,
    pub pq_lr: MintedPrekey,
    pub record: PrekeyRecordHead,
}

/// A minted slot for `subkey`: the secrets to store and the value to
/// publish.
pub struct MintedSlot {
    pub subkey: u32,
    pub opk_id: u32,
    pub opk: MintedPrekey,
    pub pq_opk_id: u32,
    pub pq_opk: MintedPrekey,
    pub record: OpkSlot,
}

/// A whole record: the head and one slot per slot subkey, every id
/// distinct within its kind.
pub struct MintedSet {
    pub head: MintedHead,
    pub slots: Vec<MintedSlot>,
}

/// The signed head over these public keys.
///
/// # Errors
/// A key has the wrong shape.
pub fn sign_head(
    identity: &Identity,
    spk_id: u32,
    spk: &[u8],
    pq_lr_id: u32,
    pq_lr: &[u8],
    issued_at_ms: i64,
) -> Result<PrekeyRecordHead, CryptoError> {
    let spk_key = x25519_from_bytes(spk).map_err(|e| shape(&e))?;
    let lr_key = ml_kem_public_from_bytes(pq_lr).map_err(|e| shape(&e))?;
    Ok(PrekeyRecordHead {
        spk_id,
        spk: spk_key.to_bytes(),
        spk_sig: sign(identity, &spk_signing_payload(spk_id, &spk_key)),
        pq_lr_id,
        pq_lr: pq_lr.to_vec(),
        pq_lr_sig: sign(
            identity,
            &pq_signing_payload(PqRole::LastResort, pq_lr_id, &lr_key),
        ),
        issued_at: issued_at_ms,
    })
}

/// The signed slot over these public keys.
///
/// # Errors
/// A key has the wrong shape.
pub fn sign_slot(
    identity: &Identity,
    opk_id: u32,
    opk: &[u8],
    pq_opk_id: u32,
    pq_opk: &[u8],
) -> Result<OpkSlot, CryptoError> {
    let opk_key = x25519_from_bytes(opk).map_err(|e| shape(&e))?;
    let pq_key = ml_kem_public_from_bytes(pq_opk).map_err(|e| shape(&e))?;
    Ok(OpkSlot {
        opk_id,
        opk: opk_key.to_bytes(),
        opk_sig: sign(identity, &opk_signing_payload(opk_id, &opk_key)),
        pq_opk_id,
        pq_opk: pq_opk.to_vec(),
        pq_opk_sig: sign(
            identity,
            &pq_signing_payload(PqRole::OneTime, pq_opk_id, &pq_key),
        ),
    })
}

/// A new head under these ids, minted at `issued_at_ms`.
#[must_use]
pub fn mint_head(
    identity: &Identity,
    rng: &dyn RandomSource,
    spk_id: u32,
    pq_lr_id: u32,
    issued_at_ms: i64,
) -> MintedHead {
    let (spk, pq_lr) = (mint_curve(rng), mint_pq(rng));
    let record = sign_head(
        identity,
        spk_id,
        &spk.public,
        pq_lr_id,
        &pq_lr.public,
        issued_at_ms,
    )
    .expect("minted keys are well-formed");
    MintedHead {
        spk_id,
        spk,
        pq_lr_id,
        pq_lr,
        record,
    }
}

/// New one-time keys under these ids for `subkey`.
#[must_use]
pub fn mint_slot(
    identity: &Identity,
    rng: &dyn RandomSource,
    subkey: u32,
    opk_id: u32,
    pq_opk_id: u32,
) -> MintedSlot {
    let (opk, pq_opk) = (mint_curve(rng), mint_pq(rng));
    let record = sign_slot(identity, opk_id, &opk.public, pq_opk_id, &pq_opk.public)
        .expect("minted keys are well-formed");
    MintedSlot {
        subkey,
        opk_id,
        opk,
        pq_opk_id,
        pq_opk,
        record,
    }
}

/// A whole record for an identity with no prekeys yet: the head and all
/// [`SLOT_COUNT`] slots, ids random and distinct per kind.
#[must_use]
pub fn mint_set(identity: &Identity, rng: &dyn RandomSource, issued_at_ms: i64) -> MintedSet {
    let mut curve_ids = BTreeSet::new();
    let mut pq_ids = BTreeSet::new();
    let fresh = |taken: &mut BTreeSet<u32>| loop {
        let id = random_id(rng);
        if taken.insert(id) {
            break id;
        }
    };
    let spk_id = fresh(&mut curve_ids);
    let pq_lr_id = fresh(&mut pq_ids);
    let head = mint_head(identity, rng, spk_id, pq_lr_id, issued_at_ms);
    let slots = (0..SLOT_COUNT)
        .map(|index| {
            let opk_id = fresh(&mut curve_ids);
            let pq_opk_id = fresh(&mut pq_ids);
            mint_slot(identity, rng, slot_subkey(index), opk_id, pq_opk_id)
        })
        .collect();
    MintedSet { head, slots }
}

fn sign(identity: &Identity, payload: &[u8]) -> [u8; 64] {
    identity.sign(payload).to_bytes()
}

fn shape(e: &crate::signal::pqxdh::PqxdhError) -> CryptoError {
    CryptoError::invalid_key(format!("prekey: {e}"))
}
