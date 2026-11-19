//! Shared private helpers for the thread pipeline.

use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::ChannelId;

use crate::deps::{ChannelMessagingDeps, OutboxItem};
use crate::error::ChannelError;
use crate::outbox_seal::open_entry_body;
use crate::send::BodyPosition;

pub(super) fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    hex::decode(hex_str)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .unwrap_or([0u8; 16])
}

/// Look up parent channel type so create_thread can pick the right
/// auto-archive default. Returns "forum_post" when parent is a forum
/// channel, "public" otherwise.
pub(super) fn parent_thread_type<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<String, ChannelError> {
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let channel = gov_state
        .channels
        .get(&ChannelId(hex_to_id_16(channel_id)))
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    Ok(if channel.channel_type == "forum" {
        "forum_post".to_string()
    } else {
        "public".to_string()
    })
}

/// The text scope a thread's replies are sealed under: its parent
/// channel's (plan E3.3 "Channel tag": a thread is a stream of its parent's
/// key, tagged by its own id).
pub(super) fn thread_scope<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    parent_channel_id: &str,
) -> Option<KeyScope> {
    let channel = ChannelId::from_hex(parent_channel_id)?;
    Some(deps.keys().scope_for_text(community_id, channel))
}

/// The replies of `thread_id`: each message entry of the thread's stream
/// across its authors' outboxes, naming its own author.
pub(super) async fn thread_replies<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
) -> Vec<(OutboxItem, ChannelMessage)> {
    (deps.read_outbox_entries(community_id, thread_id).await)
        .into_iter()
        .filter_map(|item| match &item.entry {
            ChannelRecordEntry::Message(m) if m.sender_pseudonym == hex::encode(item.author.0) => {
                let message = m.clone();
                Some((item, message))
            }
            _ => None,
        })
        .collect()
}

/// A reply's body, opened under exactly its generation of the parent
/// channel's text scope at `{anchor, thread, author, streamSeq}`. `None`
/// when that key is not held or the body does not authenticate there.
pub(super) fn open_thread_body<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
    thread_id: &str,
    item: &OutboxItem,
    message: &ChannelMessage,
) -> Option<String> {
    let at = BodyPosition {
        anchor: community_id,
        channel_id: thread_id,
        author: &item.author.0,
        stream_seq: item.stream_seq,
    };
    open_entry_body(
        &*deps.keys(),
        community_id,
        scope,
        message.mek_generation,
        at,
        &message.ciphertext,
    )
    .and_then(|plaintext| String::from_utf8(plaintext).ok())
}

/// The newest reply's commit time (seconds) and the reply count of
/// `thread_id`.
pub(super) async fn thread_activity<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
) -> (u64, u32) {
    let replies = thread_replies(deps, community_id, thread_id).await;
    let last_activity = (replies.iter())
        .map(|(item, _)| item.committed_at / 1000)
        .max()
        .unwrap_or(0);
    (
        last_activity,
        u32::try_from(replies.len()).unwrap_or(u32::MAX),
    )
}

pub(super) fn uuid_simple() -> String {
    // Lightweight v4-like 16-byte hex for thread message IDs. Matches
    // `uuid::Uuid::new_v4().simple()` output length so existing
    // src-tauri callers + DB rows can swap in cleanly.
    hex::encode(rekindle_utils::random::id_bytes_16())
}
