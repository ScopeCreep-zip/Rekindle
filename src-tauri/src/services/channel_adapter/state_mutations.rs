//! Phase 23.D.7 — small AppState read/mutation helpers extracted from
//! `deps_impl.rs` to keep the trait impl under the 500-LoC cap.
//! Sequence counters, last-send timestamp, MEK cache lookups.

use rekindle_channel::deps::ChannelMek;
use rekindle_crypto::group::media_key::MediaEncryptionKey;

use super::ChannelAdapter;

fn map_mek(mek: &MediaEncryptionKey) -> ChannelMek {
    ChannelMek {
        generation: mek.generation(),
        key_bytes: *mek.as_bytes(),
    }
}

pub(super) fn community_mek_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
) -> Option<ChannelMek> {
    adapter
        .state
        .mek_cache
        .lock()
        .get(community_id)
        .map(map_mek)
}

pub(super) fn channel_or_community_mek_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
) -> Option<ChannelMek> {
    let channel = adapter
        .state
        .channel_mek_cache
        .lock()
        .get(&(community_id.to_string(), channel_id.to_string()))
        .map(map_mek);
    channel.or_else(|| community_mek_impl(adapter, community_id))
}

pub(super) fn current_mek_generation_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
) -> Option<u64> {
    adapter
        .state
        .communities
        .read()
        .get(community_id)
        .map(|c| c.mek_generation)
}

pub(super) fn next_channel_sequence_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
) -> u64 {
    let mut communities = adapter.state.communities.write();
    let Some(community) = communities.get_mut(community_id) else {
        return 1;
    };
    let seq = community
        .channel_sequences
        .entry(channel_id.to_string())
        .or_insert(0);
    *seq += 1;
    *seq
}

pub(super) fn next_thread_sequence_impl(adapter: &ChannelAdapter, community_id: &str) -> u64 {
    let mut communities = adapter.state.communities.write();
    let Some(community) = communities.get_mut(community_id) else {
        return 1;
    };
    let seq = community
        .channel_sequences
        .entry("__thread__".into())
        .or_insert(0);
    *seq += 1;
    *seq
}

pub(super) fn mark_last_send_at_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) {
    let mut communities = adapter.state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community
            .channel_last_send_at
            .insert(channel_id.to_string(), now_ms);
    }
}
