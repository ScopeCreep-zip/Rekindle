//! Thread message send/load.

use rekindle_protocol::dht::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use super::policy::{thread_member_count, ThreadMessageView};
use super::support::{decrypt_thread_body, ensure_thread_record_and_message, thread_write_context};
use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// Phase 19.e — send a thread reply. Encrypts under the community MEK
/// with thread-specific AAD, writes to SMPL, and fan-outs the
/// `ThreadMessageReceived` envelope to the mesh.
pub async fn send_thread_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<(), ChannelError> {
    let (record_key, channel_message) =
        ensure_thread_record_and_message(deps, community_id, thread_id, body).await?;

    // Reuse channel_write_context's channel-key plumbing pattern via a
    // temporary context — the slot bits come from community state but
    // the channel_key is the thread's record_key.
    let parent_context = thread_write_context(deps, community_id, &record_key)?;
    deps.write_channel_message_smpl(&parent_context, &channel_message)
        .await?;

    let envelope = CommunityEnvelope::Control(ControlPayload::ThreadMessageReceived {
        thread_id: thread_id.to_string(),
        message_id: channel_message.message_id.clone().unwrap_or_default(),
        sender_pseudonym: channel_message.sender_pseudonym.clone(),
        ciphertext: channel_message.ciphertext.clone(),
        mek_generation: channel_message.mek_generation,
        timestamp: channel_message.timestamp / 1000,
        reply_to_id: None,
    });
    deps.send_to_mesh(community_id, &envelope)?;
    Ok(())
}

/// Phase 19.e — load thread messages from the thread's SMPL record,
/// decrypt with AAD waterfall, and return rendered `ThreadMessageView`s.
pub async fn load_thread_messages<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    limit: u32,
    before_timestamp_secs: Option<u64>,
) -> Result<Vec<ThreadMessageView>, ChannelError> {
    let thread = deps
        .thread_state(community_id, thread_id)
        .ok_or_else(|| ChannelError::Adapter("thread not found".into()))?;
    let Some(record_key) = thread.record_key.clone() else {
        return Ok(Vec::new());
    };
    let entries = deps
        .read_all_channel_entries(&record_key, thread_member_count())
        .await?;
    let mut items: Vec<(u32, ChannelMessage)> = entries
        .into_iter()
        .filter_map(|item| match item.entry {
            ChannelRecordEntry::Message(msg) => Some((item.subkey_index, msg)),
            _ => None,
        })
        .collect();
    items.sort_by(|a, b| {
        a.1.lamport_ts
            .cmp(&b.1.lamport_ts)
            .then_with(|| a.1.sender_pseudonym.cmp(&b.1.sender_pseudonym))
    });
    let before_ms = before_timestamp_secs.map_or(u64::MAX, |ts| ts.saturating_mul(1000));
    let my_pseudonym = deps.my_pseudonym_hex(community_id).unwrap_or_default();

    let mut messages: Vec<ThreadMessageView> = items
        .into_iter()
        .filter(|(_, message)| message.timestamp < before_ms)
        .rev()
        .take(limit.min(200) as usize)
        .map(|(subkey_index, message)| {
            let body = decrypt_thread_body(
                deps,
                community_id,
                &record_key,
                subkey_index,
                message.lamport_ts,
                &message.ciphertext,
                message.mek_generation,
            );
            ThreadMessageView {
                is_own: message.sender_pseudonym == my_pseudonym,
                sender_pseudonym: message.sender_pseudonym.clone(),
                body,
                timestamp_ms: message.timestamp,
                server_message_id: message.message_id.clone(),
                mek_generation: message.mek_generation,
                subkey_index,
                lamport_ts: message.lamport_ts,
            }
        })
        .collect();
    messages.reverse();
    Ok(messages)
}
