//! Which of our posts no reader keeps after our removal (plan E3.2 S9,
//! F9-3): the entries of our slot past the head the removal recorded.

use std::collections::BTreeSet;

use rekindle_codec::community::channel_record::{
    encode_page_entries, ChannelMessage, ChannelRecordEntry,
};
use rekindle_secrets::slot_head::slot_head_digest;
use rekindle_types::id::PseudonymKey;

use super::{entry_message_id, first_past_head, OwnPage};

const RECORD: &str = "VLD0:channel";
const SLOT: u32 = 3;

fn key() -> rekindle_secrets::ed25519_dalek::SigningKey {
    rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[7; 32])
}

fn me() -> PseudonymKey {
    PseudonymKey(key().verifying_key().to_bytes())
}

fn post(n: u64) -> ChannelRecordEntry {
    ChannelRecordEntry::Message(ChannelMessage {
        sequence: n,
        sender_pseudonym: hex::encode(me().0),
        ciphertext: vec![u8::try_from(n).unwrap_or(0); 8],
        mek_generation: 1,
        timestamp: 1_000 + n,
        reply_to: None,
        lamport_ts: n,
        message_id: Some(format!("msg_{n}")),
        attachment: None,
        flags: 0,
        mentioned_pseudonyms: Vec::new(),
        mentioned_roles: Vec::new(),
    })
}

fn encode(entries: &[ChannelRecordEntry]) -> Option<Vec<u8>> {
    encode_page_entries(me(), &key(), entries.to_vec()).ok()
}

/// Our slot after `count` appends: the value at seq `count`, as the record
/// pool returns it decrypted.
fn page(count: u64) -> OwnPage {
    let entries: Vec<_> = (1..=count).map(post).collect();
    OwnPage {
        seq: u32::try_from(count).unwrap_or(0),
        data: encode(&entries).unwrap_or_default(),
        entries,
    }
}

/// The head a remover records of our slot after `count` appends.
fn head_after(count: u64) -> (u32, [u8; 32]) {
    let held = page(count);
    (
        held.seq,
        slot_head_digest(RECORD, SLOT, held.seq, &held.data),
    )
}

fn not_kept(page: &OwnPage, heads: &BTreeSet<(u32, [u8; 32])>) -> Option<Vec<String>> {
    let kept = first_past_head(RECORD, SLOT, page, heads, encode)?;
    Some(
        page.entries[kept..]
            .iter()
            .filter_map(entry_message_id)
            .collect(),
    )
}

#[test]
fn a_post_written_after_the_kicks_head_is_not_kept_and_one_before_is() {
    // The kick headed our slot after two posts; we wrote a third before
    // we merged it (A/B round 9 step 4, ab10-pop2).
    let heads = BTreeSet::from([head_after(2)]);
    assert_eq!(not_kept(&page(3), &heads), Some(vec!["msg_3".to_owned()]));
}

#[test]
fn nothing_written_past_the_head_keeps_every_post() {
    let heads = BTreeSet::from([head_after(3)]);
    assert_eq!(not_kept(&page(3), &heads), Some(Vec::new()));
}

#[test]
fn a_head_of_another_slot_or_page_matches_nothing() {
    // Our old slot's head from an earlier leave (F8-1 keeps every
    // departure's heads): same record, another subkey.
    let old = page(2);
    let other_slot = (
        old.seq,
        slot_head_digest(RECORD, SLOT + 1, old.seq, &old.data),
    );
    assert_eq!(not_kept(&page(3), &BTreeSet::from([other_slot])), None);
    assert_eq!(not_kept(&page(3), &BTreeSet::new()), None);
}

#[test]
fn the_latest_reproducible_head_decides() {
    // A leave's head after one post and a kick's after two, both of this
    // slot: the later one keeps more.
    let heads = BTreeSet::from([head_after(1), head_after(2)]);
    assert_eq!(
        not_kept(&page(4), &heads),
        Some(vec!["msg_3".to_owned(), "msg_4".to_owned()])
    );
}
