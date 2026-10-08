//! Phase 23.D.7 — small AppState read/mutation helpers extracted from
//! `deps_impl.rs` to keep the trait impl under the 500-LoC cap.
//! Sequence counters and last-send timestamp. Keys are read through
//! `ChannelMessagingDeps::keys` (`state_helpers::key_provider`).

use super::ChannelAdapter;

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
