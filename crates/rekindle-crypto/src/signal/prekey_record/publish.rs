//! What our record is written with, and the pure decisions about it
//! (plan E2.4; audit 2026-10-20 X-4): the packed value of each subkey, the
//! values a minted set publishes, which of a pass's values are written
//! before the profile names the record, and what the status read calls a
//! broken maintenance. The storage keeps the rows; the transport writes
//! these values.

use rekindle_codec::capnp_codec::identity::{
    encode_opk_slot, encode_prekey_head, OpkSlot, PrekeyRecordHead,
};
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
    #[must_use]
    pub fn head(head: &PrekeyRecordHead) -> Self {
        Self {
            subkey: HEAD,
            value: encode_prekey_head(head),
        }
    }

    /// A minted slot's value.
    #[must_use]
    pub fn slot(slot: &MintedSlot) -> Self {
        Self::signed_slot(slot.subkey, &slot.record)
    }

    /// The value of a slot signed from stored keys, at `subkey`.
    #[must_use]
    pub fn signed_slot(subkey: u32, slot: &OpkSlot) -> Self {
        Self {
            subkey,
            value: encode_opk_slot(slot),
        }
    }
}

/// The values a minted set publishes: the head, then every slot.
#[must_use]
pub fn set_values(set: &MintedSet) -> Vec<Publish> {
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
mod tests {
    use super::*;
    use rekindle_types::dht_layout::prekeys::slot_subkey;

    fn healthy() -> PrekeyStatus {
        PrekeyStatus {
            device_id: None,
            record_key: None,
            spk_id: Some(1),
            spk_age_secs: Some(3_600),
            pq_lr_id: Some(2),
            lr_age_secs: Some(3_600),
            fresh_slots: SLOT_COUNT,
            consumed_slots: 0,
            component_lens: None,
        }
    }

    #[test]
    fn the_problem_names_what_maintenance_failed_to_keep() {
        assert_eq!(prekey_problem(&healthy()), None);
        let low = PrekeyStatus {
            fresh_slots: 3,
            ..healthy()
        };
        assert_eq!(
            prekey_problem(&low),
            Some(format!("3/{SLOT_COUNT} fresh slots"))
        );
        let stale = PrekeyStatus {
            spk_age_secs: Some(15 * 86_400),
            ..healthy()
        };
        assert!(prekey_problem(&stale).unwrap().contains("15 days"));
        let none = PrekeyStatus {
            spk_age_secs: None,
            ..healthy()
        };
        assert!(prekey_problem(&none).is_some());
    }

    fn value(subkey: u32) -> Publish {
        Publish {
            subkey,
            value: vec![u8::try_from(subkey % 256).unwrap()],
        }
    }

    #[test]
    fn the_first_contact_part_is_the_head_and_the_watermark_slots() {
        let slots = u32::try_from(LOW_SLOT_WATERMARK).unwrap() + 5;
        let mut values: Vec<Publish> = (0..slots).map(|i| value(slot_subkey(i))).collect();
        values.insert(3, value(HEAD));
        let (now, later) = split_first_contact(values);
        assert_eq!(now.len(), 1 + LOW_SLOT_WATERMARK);
        assert!(now.iter().any(|p| p.subkey == HEAD));
        assert_eq!(later.len(), 5);
        assert!(later.iter().all(|p| p.subkey != HEAD));

        let (now, later) = split_first_contact(vec![value(slot_subkey(0))]);
        assert_eq!((now.len(), later.len()), (1, 0));
    }
}
