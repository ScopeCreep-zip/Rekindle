//! Channel record reads: watching and merged multi-writer history.

use super::codec::{decode_channel_entries, message_from_entry};
use super::types::{ChannelMessage, ChannelRecordItem};
use super::CHANNEL_OWNER_SUBKEY_COUNT;
use crate::dht::DHTManager;
use crate::error::ProtocolError;

/// Watch a channel record for new messages.
pub async fn watch_channel(
    dht: &DHTManager,
    key: &str,
    subkey_count: u32,
) -> Result<bool, ProtocolError> {
    let subkeys: Vec<u32> = (0..subkey_count).collect();
    dht.watch_record(key, &subkeys).await
}

// ── SMPL multi-writer channel persistence ──

/// Decode all durable entries from all member subkeys in the channel SMPL record.
pub async fn read_all_channel_entries(
    rc: &veilid_core::RoutingContext,
    channel_key: &str,
    member_count: u32,
) -> Result<Vec<ChannelRecordItem>, ProtocolError> {
    use futures::stream::{FuturesUnordered, StreamExt};

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
    let mut futs = FuturesUnordered::new();

    for i in 0..member_count {
        let sem = sem.clone();
        let rc = rc.clone();
        let key = channel_key.to_string();
        let subkey = u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + i;
        futs.push(async move {
            let permit = sem.acquire().await.unwrap();
            let mgr = DHTManager::new(rc);
            let result = mgr.get_value(&key, subkey).await;
            drop(permit);
            (subkey, result)
        });
    }

    let mut items = Vec::new();
    while let Some((subkey_index, result)) = futs.next().await {
        if let Ok(Some(data)) = result {
            if let Ok(entries) = decode_channel_entries(&data) {
                items.extend(entries.into_iter().map(|entry| ChannelRecordItem {
                    subkey_index,
                    entry,
                }));
            }
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
/// ordering. Uses parallel reads bounded by a semaphore.
pub async fn read_all_channel_messages(
    rc: &veilid_core::RoutingContext,
    channel_key: &str,
    member_count: u32,
) -> Result<Vec<ChannelMessage>, ProtocolError> {
    let mut all_messages: Vec<ChannelMessage> =
        read_all_channel_entries(rc, channel_key, member_count)
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
