//! Channel record reads: watching and merged multi-writer history.

use super::codec::{decode_channel_entries, message_from_entry};
use super::types::{ChannelMessage, ChannelRecordItem};
use super::CHANNEL_OWNER_SUBKEY_COUNT;
use crate::dht::parse_record_key;
use crate::dht::pool::RecordPool;
use crate::error::ProtocolError;

// A channel watch is the record pool's (`RecordPool::watch` on a lease,
// plan C7.4); nothing here watches.

// ── SMPL multi-writer channel persistence ──

/// Decode the durable entries of the members in `member_slots` (slot
/// indices, the writer index the caller's membership state vouches for).
/// One inspect over those slots, then a read of each slot that holds a
/// value, from the network only when it changed (`RecordPool::read_changed`,
/// plan C7.12); a slot outside the index is never read.
pub async fn read_all_channel_entries(
    pool: &RecordPool,
    channel_key: &str,
    member_slots: &[u32],
) -> Result<Vec<ChannelRecordItem>, ProtocolError> {
    let subkeys: Vec<u32> = member_slots
        .iter()
        .map(|slot| u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + slot)
        .collect();
    let lease = pool.acquire(&parse_record_key(channel_key)?, None).await?;
    let read = pool.read_changed(lease, &subkeys).await;
    pool.release(lease).await;

    let mut items = Vec::new();
    for (subkey_index, data) in read? {
        if let Ok(entries) = decode_channel_entries(data.data()) {
            items.extend(entries.into_iter().map(|entry| ChannelRecordItem {
                subkey_index,
                entry,
            }));
        }
    }
    items.sort_by(|a, b| {
        a.entry
            .lamport()
            .cmp(&b.entry.lamport())
            .then_with(|| a.subkey_index.cmp(&b.subkey_index))
    });
    Ok(items)
}

/// Read all messages from all member subkeys in the channel SMPL record.
///
/// Returns messages sorted by (lamport_ts, sender_pseudonym) for deterministic
/// ordering. Reads as [`read_all_channel_entries`].
pub async fn read_all_channel_messages(
    pool: &RecordPool,
    channel_key: &str,
    member_slots: &[u32],
) -> Result<Vec<ChannelMessage>, ProtocolError> {
    let mut all_messages: Vec<ChannelMessage> =
        read_all_channel_entries(pool, channel_key, member_slots)
            .await?
            .into_iter()
            .filter_map(|item| message_from_entry(&item.entry).cloned())
            .collect();

    all_messages.sort_by(|a, b| {
        a.lamport_ts
            .cmp(&b.lamport_ts)
            .then_with(|| a.sender_pseudonym.cmp(&b.sender_pseudonym))
    });

    Ok(all_messages)
}
