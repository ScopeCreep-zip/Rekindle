//! Phase 23.D.7 — voice-stage hand-raise SMPL operations ported from
//! `src-tauri/services/community/stage.rs`. Persists each raise/lower
//! to the channel record as a `ChannelHandRaise` entry; the reader
//! aggregates the latest state per slot, mapping subkey indices back
//! to pseudonyms via the adapter's `stage_pseudonyms_by_subkey` lookup.

use std::collections::HashMap;

use rekindle_protocol::dht::community::channel_record::{ChannelHandRaise, ChannelRecordEntry};
use rekindle_records::schema::MAX_MEMBERS_PER_SEGMENT;

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

pub async fn persist_hand_raise<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    raised: bool,
) -> Result<(), ChannelError> {
    let context = deps.channel_write_context(community_id, channel_id)?;
    let hand_raise = ChannelHandRaise {
        raised,
        lamport: deps.increment_lamport(community_id),
    };
    deps.write_channel_hand_raise_smpl(&context, &hand_raise)
        .await
}

pub async fn list_hand_raises<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<String>, ChannelError> {
    let context = deps.channel_write_context(community_id, channel_id)?;
    let entries = deps
        .read_all_channel_entries(
            &context.channel_key,
            u32::try_from(MAX_MEMBERS_PER_SEGMENT).unwrap_or(u32::MAX),
        )
        .await?;

    let pseudonyms_by_subkey = deps.stage_pseudonyms_by_subkey(community_id).await?;
    let mut latest_by_subkey = HashMap::<u32, (u64, bool)>::new();
    for item in entries {
        let ChannelRecordEntry::HandRaise(hand_raise) = item.entry else {
            continue;
        };
        let replace = latest_by_subkey
            .get(&item.subkey_index)
            .is_none_or(|(lamport, _)| hand_raise.lamport >= *lamport);
        if replace {
            latest_by_subkey.insert(item.subkey_index, (hand_raise.lamport, hand_raise.raised));
        }
    }

    let mut raised = latest_by_subkey
        .into_iter()
        .filter_map(|(subkey_index, (_, is_raised))| {
            is_raised
                .then(|| pseudonyms_by_subkey.get(&subkey_index).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    raised.sort();
    Ok(raised)
}
