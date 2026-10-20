//! The prekey record's pure side (plan E2.4, D18): its ids, minting and
//! signing a head and its slots, and the two decisions the hosts run over
//! it, so a seeded [`RandomSource`] and a fixed clock fix every outcome
//! (the C9 scenarios).
//!
//! - [`plan_maintenance`] (B1): what our own record needs now. A consumed
//!   slot is refilled; when fewer than [`LOW_SLOT_WATERMARK`] slots are
//!   fresh every empty one is filled (a batch back to [`SLOT_COUNT`]);
//!   a slot older than [`OPK_STALE_MS`] is replaced; the head (the signed
//!   prekey and the ML-KEM last-resort key) rotates every
//!   [`SIGNED_ROTATE_AFTER_MS`], so no published head reaches
//!   [`SIGNED_MAX_AGE_MS`] while maintenance runs (Signal-Android
//!   `PreKeysSyncJob`, `PreKeyUtil`). Replaced keys stay usable for
//!   [`RETIRED_ARCHIVE_MS`]; the storage's archive sweep then deletes them.
//! - [`pick_slot`] (B2): which slot of a peer's record to initiate from,
//!   uniformly among the slots it lists that we have not used, else the
//!   head alone (PQXDH §2.5, §3.3).
//!
//! Every id is a random non-zero u31 (PQXDH §4.13, [`random_id`]); the
//! storage keeps them unique per kind.

mod mint;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use rekindle_codec::capnp_codec::identity::{OpkSlot, PrekeyRecordHead};
use rekindle_types::dht_layout::prekeys::{is_slot, slot_subkey, SLOT_COUNT};
use rekindle_utils::random::RandomSource;

use crate::error::CryptoError;
use crate::signal::prekeys::PreKeyBundle;

pub use mint::{
    mint_head, mint_set, mint_slot, sign_head, sign_slot, MintedHead, MintedSet, MintedSlot,
};

const DAY_MS: i64 = 86_400_000;

/// The largest prekey id: ids are u31, so they fit every signed or
/// unsigned 32-bit field a peer may read them into.
pub const PREKEY_ID_MAX: u32 = 0x7FFF_FFFF;
/// The head rotates this long after it was minted.
pub const SIGNED_ROTATE_AFTER_MS: i64 = 2 * DAY_MS;
/// No head older than this is published; past it maintenance is failing
/// (the status check warns).
pub const SIGNED_MAX_AGE_MS: i64 = 14 * DAY_MS;
/// A replaced or consumed key's row is kept this long, so an init in
/// flight still opens; then the storage deletes it.
pub const RETIRED_ARCHIVE_MS: i64 = 30 * DAY_MS;
/// A published one-time slot no peer claimed is replaced after this.
pub const OPK_STALE_MS: i64 = 90 * DAY_MS;
/// Below this many fresh slots every empty slot is filled.
pub const LOW_SLOT_WATERMARK: usize = 10;
/// B5: at most this many `PreKeyRejected` per sender per
/// [`REJECTION_WINDOW_MS`].
pub const REJECTIONS_PER_WINDOW: u32 = 10;
/// B5's rate window.
pub const REJECTION_WINDOW_MS: i64 = 3 * 3_600_000;

/// Whether `id` is a prekey id: non-zero and within u31.
#[must_use]
pub fn is_prekey_id(id: u32) -> bool {
    id != 0 && id <= PREKEY_ID_MAX
}

/// A random prekey id (PQXDH §4.13). The caller checks it is unused for
/// its kind.
#[must_use]
pub fn random_id(rng: &dyn RandomSource) -> u32 {
    loop {
        let id = rng.next_u32() & PREKEY_ID_MAX;
        if id != 0 {
            return id;
        }
    }
}

/// B5's random delay before a `PreKeyRejected` goes, in milliseconds:
/// uniform in 2–10 s, so its timing says nothing about our state.
#[must_use]
pub fn rejection_delay_ms(rng: &dyn RandomSource) -> u64 {
    2_000 + rng.below(8_001)
}

/// The hash of an initiator's base key that a `PreKeyRejected` names
/// (B5): SHA-256 over the domain label and the key, so the rejection
/// matches one pending init without carrying the key itself.
#[must_use]
pub fn base_key_hash(base_key: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(rekindle_types::domains::PREKEY_REJECTED_BASE_KEY.as_bytes());
    hasher.update(base_key);
    hasher.finalize().into()
}

/// Our current head, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalHead {
    pub spk_id: u32,
    pub pq_lr_id: u32,
    /// When the head was minted (its `issuedAt`).
    pub issued_at_ms: i64,
}

/// One slot of our record, as stored. A slot with no row is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Its one-time keys are unclaimed.
    Live { created_at_ms: i64 },
    /// Its keys were consumed and nothing has replaced them.
    Consumed,
}

/// Our record as storage holds it: the head and each slot's state, keyed
/// by subkey (absent: empty).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalPrekeys {
    pub head: Option<LocalHead>,
    pub slots: BTreeMap<u32, SlotState>,
}

/// One change maintenance makes to our record. Storage applies each in
/// one transaction and returns the bytes to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrekeyAction {
    /// Mint a new signed prekey and last-resort key, retiring the current.
    RotateHead,
    /// Mint new one-time keys into `subkey`, retiring any it holds.
    FillSlot { subkey: u32 },
}

/// B1: what our record needs at `now_ms`, head first, then slots in
/// subkey order.
#[must_use]
pub fn plan_maintenance(now_ms: i64, local: &LocalPrekeys) -> Vec<PrekeyAction> {
    let mut actions = Vec::new();
    let head_due = local
        .head
        .is_none_or(|head| now_ms - head.issued_at_ms >= SIGNED_ROTATE_AFTER_MS);
    if head_due {
        actions.push(PrekeyAction::RotateHead);
    }
    let mut fresh = 0;
    let mut empty = Vec::new();
    for index in 0..SLOT_COUNT {
        let subkey = slot_subkey(index);
        match local.slots.get(&subkey) {
            Some(SlotState::Live { created_at_ms }) if now_ms - created_at_ms < OPK_STALE_MS => {
                fresh += 1;
            }
            Some(SlotState::Live { .. } | SlotState::Consumed) => {
                actions.push(PrekeyAction::FillSlot { subkey });
                fresh += 1;
            }
            None => empty.push(subkey),
        }
    }
    if fresh < LOW_SLOT_WATERMARK {
        actions.extend(
            empty
                .into_iter()
                .map(|subkey| PrekeyAction::FillSlot { subkey }),
        );
        actions.sort_unstable();
    }
    actions
}

/// The one-time ids we have used against one peer device
/// (`prekey_slots_used`).
pub type UsedIds = BTreeSet<u32>;

/// A peer device's prekey record as read, checked under its **pinned**
/// identity: the head verified, and only the slots whose signatures
/// verify kept (an unreadable slot is one the pick cannot use).
#[derive(Debug, Clone)]
pub struct RecordView {
    identity_key: [u8; 32],
    head: PrekeyRecordHead,
    slots: Vec<(u32, OpkSlot)>,
}

impl RecordView {
    /// Check `head` and `slots` (subkey, slot) under `identity_key`.
    ///
    /// # Errors
    /// The head does not verify under `identity_key`: the record is not
    /// the pinned identity's, and nothing in it is usable.
    pub fn new(
        identity_key: &[u8; 32],
        head: PrekeyRecordHead,
        slots: impl IntoIterator<Item = (u32, OpkSlot)>,
    ) -> Result<Self, CryptoError> {
        PreKeyBundle::from_record(identity_key, &head, None)?;
        let slots = slots
            .into_iter()
            .filter(|(subkey, slot)| {
                is_slot(*subkey)
                    && PreKeyBundle::from_record(identity_key, &head, Some(slot)).is_ok()
            })
            .collect();
        Ok(Self {
            identity_key: *identity_key,
            head,
            slots,
        })
    }

    /// The verified head.
    pub fn head(&self) -> &PrekeyRecordHead {
        &self.head
    }

    /// The verified slots, as `(subkey, slot)`.
    pub fn slots(&self) -> &[(u32, OpkSlot)] {
        &self.slots
    }

    /// The one-time ids the record lists: a `prekey_slots_used` row for
    /// any other id is swept.
    pub fn listed_opk_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().map(|(_, slot)| slot.opk_id)
    }

    /// The bundle `pick` names.
    ///
    /// # Errors
    /// `pick` names a slot this record does not hold.
    pub fn bundle(&self, pick: &Pick) -> Result<PreKeyBundle, CryptoError> {
        let slot = match pick {
            Pick::LastResortOnly => None,
            Pick::Slot { subkey, opk_id } => Some(
                self.slots
                    .iter()
                    .find(|(s, slot)| s == subkey && slot.opk_id == *opk_id)
                    .map(|(_, slot)| slot)
                    .ok_or_else(|| {
                        CryptoError::invalid_key(format!("no slot {subkey} with id {opk_id}"))
                    })?,
            ),
        };
        PreKeyBundle::from_record(&self.identity_key, &self.head, slot)
    }
}

/// B2's choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Initiate with this slot's one-time keys.
    Slot { subkey: u32, opk_id: u32 },
    /// No slot is usable: the signed prekey and the last-resort key alone.
    LastResortOnly,
}

/// B2: a slot uniformly among those `view` lists whose one-time id is not
/// in `used`, else [`Pick::LastResortOnly`].
#[must_use]
pub fn pick_slot(view: &RecordView, used: &UsedIds, rng: &dyn RandomSource) -> Pick {
    let candidates: Vec<_> = view
        .slots
        .iter()
        .filter(|(_, slot)| !used.contains(&slot.opk_id))
        .collect();
    if candidates.is_empty() {
        return Pick::LastResortOnly;
    }
    let bound = u64::try_from(candidates.len()).expect("at most SLOT_COUNT candidates");
    let index = usize::try_from(rng.below(bound)).expect("an index below the candidate count");
    let (subkey, slot) = candidates[index];
    Pick::Slot {
        subkey: *subkey,
        opk_id: slot.opk_id,
    }
}
