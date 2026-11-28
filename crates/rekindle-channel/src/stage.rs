//! Voice-stage hand raises (plan E3.3): a raise or lower is an op on the
//! member's channel outbox (`M`/4), queued on the stage channel's
//! `channel_post` stream; readers keep each member's latest by its stream
//! seq. The voice signaling gossip stays the live path.

use std::collections::HashMap;

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_types::message::{ChannelPost, PostEntry};

use crate::deps::{ChannelMessagingDeps, OutboxItem};
use crate::error::ChannelError;

/// Queue our hand `raised` on `channel_id`'s stage.
///
/// # Errors
/// The channel is unknown or the queue failed.
pub async fn persist_hand_raise<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    raised: bool,
) -> Result<(), ChannelError> {
    deps.channel_info(community_id, channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    let post = ChannelPost {
        governance_cut: Vec::new(),
        community: community_id.to_owned(),
        channel: channel_id.to_owned(),
        message: format!(
            "hand:{}",
            hex::encode(rekindle_utils::random::id_bytes_16())
        ),
        entry: PostEntry::HandRaise { raised },
    };
    deps.queue_post(&post).await
}

/// The pseudonyms whose hand is raised on `channel_id`'s stage, sorted.
///
/// # Errors
/// The channel is unknown.
pub async fn list_hand_raises<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<String>, ChannelError> {
    deps.channel_info(community_id, channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    Ok(raised_hands(
        &deps.read_outbox_entries(community_id, channel_id).await,
    ))
}

/// Each member's latest hand raise among `entries` by stream seq: the
/// pseudonyms (hex) whose hand stands raised, sorted.
#[must_use]
pub fn raised_hands(entries: &[OutboxItem]) -> Vec<String> {
    let mut latest = HashMap::<String, (u64, bool)>::new();
    for item in entries {
        let ChannelRecordEntry::HandRaise(hand) = &item.entry else {
            continue;
        };
        let author = hex::encode(item.author.0);
        if latest
            .get(&author)
            .is_none_or(|(seq, _)| item.stream_seq > *seq)
        {
            latest.insert(author, (item.stream_seq, hand.raised));
        }
    }
    let mut raised: Vec<String> = (latest.into_iter())
        .filter_map(|(author, (_, raised))| raised.then_some(author))
        .collect();
    raised.sort();
    raised
}

#[cfg(test)]
#[path = "stage_tests.rs"]
mod tests;
