//! What our record is written with, and the pure decisions about it
//! (plan E2.4; audit 2026-10-20 X-4): the packed value of each subkey, the
//! values a minted set publishes, which of a pass's values are written
//! before the profile names the record, and what the status read calls a
//! broken maintenance. The storage keeps the rows; the transport writes
//! these values.

use rekindle_codec::capnp_codec::identity::{
    encode_opk_slot, encode_prekey_head, OpkSlot, PrekeyRecordHead,
};
use rekindle_codec::CodecError;
use rekindle_types::dht_layout::prekeys::{HEAD, SLOT_COUNT};
use rekindle_types::dto::identity::PrekeyStatus;

use super::{MintedSet, MintedSlot, LOW_SLOT_WATERMARK, SIGNED_MAX_AGE_MS};

/// One value to write to our prekey record: the packed `PrekeyRecordHead`
/// at [`HEAD`] or an `OpkSlot` at a slot subkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    pub subkey: u32,
    pub value: Vec<u8>,
}

impl Publish {
    /// The head's value.
    pub fn head(head: &PrekeyRecordHead) -> Result<Self, CodecError> {
        Ok(Self {
            subkey: HEAD,
            value: encode_prekey_head(head)?,
        })
    }

    /// A minted slot's value.
    pub fn slot(slot: &MintedSlot) -> Result<Self, CodecError> {
        Self::signed_slot(slot.subkey, &slot.record)
    }

    /// The value of a slot signed from stored keys, at `subkey`.
    pub fn signed_slot(subkey: u32, slot: &OpkSlot) -> Result<Self, CodecError> {
        Ok(Self {
            subkey,
            value: encode_opk_slot(slot)?,
        })
    }
}

/// The values a minted set publishes: the head, then every slot.
pub fn set_values(set: &MintedSet) -> Result<Vec<Publish>, CodecError> {
    std::iter::once(Publish::head(&set.head.record))
        .chain(set.slots.iter().map(Publish::slot))
        .collect()
}

/// Split a pass's values at the unlock: the head and the first
/// [`LOW_SLOT_WATERMARK`] slots are written before the profile names the
/// record, so a friend's first contact finds keys; the rest after it, so a
/// new identity's 101 writes do not hold its profile back. Order within
/// each part is kept.
#[must_use]
pub fn split_first_contact(values: Vec<Publish>) -> (Vec<Publish>, Vec<Publish>) {
    let mut slots_now = 0;
    values.into_iter().partition(|p| {
        if p.subkey == HEAD {
            return true;
        }
        slots_now += 1;
        slots_now <= LOW_SLOT_WATERMARK
    })
}

/// Why the maintenance looks broken, if it does: no head yet, fewer fresh
/// slots than the watermark, or a head past [`SIGNED_MAX_AGE_MS`].
#[must_use]
pub fn prekey_problem(status: &PrekeyStatus) -> Option<String> {
    let (Some(spk), Some(lr)) = (status.spk_age_secs, status.lr_age_secs) else {
        return Some("no prekeys published yet".to_string());
    };
    if usize::try_from(status.fresh_slots).unwrap_or(usize::MAX) < LOW_SLOT_WATERMARK {
        return Some(format!("{}/{SLOT_COUNT} fresh slots", status.fresh_slots));
    }
    let oldest = spk.max(lr);
    let limit = u64::try_from(SIGNED_MAX_AGE_MS / 1000).unwrap_or(u64::MAX);
    (oldest > limit).then(|| format!("signed prekeys {} days old", oldest / 86_400))
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
