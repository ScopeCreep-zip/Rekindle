//! Phase 19.g-REDO — channel send/forward orchestrators.
//!
//! Ported from src-tauri/services/community/channel_messages.rs.
//! Crate-side `send_channel_message` and `forward_channel_message`
//! parameterised over `D: ChannelMessagingDeps`. The retry worker
//! itself (loop reading `channel_write_retry_tx`) stays src-tauri
//! because it owns the receiver end of the channel; the crate
//! exposes the per-write retry-enqueue primitive via the deps trait.

use rekindle_protocol::dht::community::channel_record::{ChannelForward, ChannelMessage};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::permissions::BYPASS_SLOWMODE;

use crate::deps::{
    ChannelMessagingDeps, ChannelSendOutcome, ChannelWriteContext, PendingChannelWrite,
    SentChannelMessageEcho,
};
use crate::error::ChannelError;
use crate::mentions::resolve_outbound_mentions;
use crate::send::{
    build_channel_message, channel_message_subkey, encrypt_channel_body, slowmode_check,
};

/// Architecture §28.7 — slowmode gate that combines the pure
/// `slowmode_check` decision with the `BYPASS_SLOWMODE` permission
/// shortcut. Adapter supplies the channel snapshot + permission bits.
pub fn enforce_slowmode_with_bypass<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) -> Result<(), ChannelError> {
    let Some(channel) = deps.channel_info(community_id, channel_id) else {
        return Ok(());
    };
    let Some(seconds) = channel.slowmode_seconds.filter(|&s| s > 0) else {
        return Ok(());
    };
    let perms = deps.compute_my_permissions(community_id);
    if perms & BYPASS_SLOWMODE == BYPASS_SLOWMODE {
        return Ok(());
    }
    let last_send = u64::try_from(channel.last_send_at_ms.unwrap_or(0)).unwrap_or(0);
    let now = u64::try_from(now_ms).unwrap_or(0);
    slowmode_check(Some(seconds), last_send, now).map_err(|e| match e {
        ChannelError::SlowmodeActive { wait_ms } => {
            let remaining_secs = wait_ms.div_ceil(1000);
            ChannelError::Adapter(format!(
                "slowmode active — wait {remaining_secs}s before sending again"
            ))
        }
        other => other,
    })
}

/// Build the `CommunityEnvelope::MessageNotification` for a send.
fn build_message_notification(
    channel_id: &str,
    channel_msg: &ChannelMessage,
    slot_index: u32,
) -> Result<CommunityEnvelope, ChannelError> {
    Ok(CommunityEnvelope::MessageNotification {
        channel_id: channel_id.to_string(),
        message_id: channel_msg
            .message_id
            .clone()
            .ok_or_else(|| ChannelError::Adapter("channel message missing message_id".into()))?,
        author_pseudonym: channel_msg.sender_pseudonym.clone(),
        subkey_index: channel_message_subkey(slot_index),
        lamport_ts: channel_msg.lamport_ts,
        sequence: channel_msg.sequence,
        content_hash: blake3::hash(&channel_msg.ciphertext)
            .to_hex()
            .to_string(),
        timestamp: channel_msg.timestamp,
    })
}

fn build_forward_notification(
    channel_id: &str,
    forward: &ChannelForward,
    slot_index: u32,
) -> Result<CommunityEnvelope, ChannelError> {
    Ok(CommunityEnvelope::MessageNotification {
        channel_id: channel_id.to_string(),
        message_id: forward
            .message_id
            .clone()
            .ok_or_else(|| ChannelError::Adapter("forward missing message_id".into()))?,
        author_pseudonym: forward.sender_pseudonym.clone(),
        subkey_index: channel_message_subkey(slot_index),
        lamport_ts: forward.lamport_ts,
        sequence: forward.sequence,
        content_hash: blake3::hash(&forward.content_snapshot).to_hex().to_string(),
        timestamp: forward.timestamp,
    })
}

fn random_message_id(prefix: &str) -> String {
    let bytes: [u8; 16] = rand::random();
    format!("{prefix}{}", hex::encode(bytes))
}

/// Send-result returned to the orchestrator caller.
#[derive(Debug, Clone)]
pub struct ChannelSendResult {
    pub status: String,
    pub message_id: String,
    pub sender_pseudonym: String,
    pub timestamp_ms: u64,
    pub body: String,
}

/// Phase 19.g — full send_channel_message pipeline.
///
/// Architecture §8 / §15.4 / §28.5 / §28.7:
/// - rejects forum channels (posts go through thread creation)
/// - ensures a Plate Gate channel-segment record exists
/// - enforces SEND_MESSAGES permission
/// - enforces slowmode (with BYPASS_SLOWMODE shortcut)
/// - encrypts with AAD-bound MEK
/// - persists to local DB + bumps channel sequence + records slowmode
/// - resolves cleartext mentions for notification routing
/// - writes to SMPL channel record (enqueues retry on failure)
/// - gossips MessageNotification
/// - emits a local chat-event echo
pub async fn send_channel_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    body: &str,
) -> Result<ChannelSendResult, ChannelError> {
    let timestamp_ms = i64::try_from(rekindle_utils::timestamp_secs() * 1000).unwrap_or(i64::MAX);

    let info = deps
        .channel_info(community_id, channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    if info.is_forum {
        return Err(ChannelError::Adapter(
            "forum channels accept posts only through thread creation".into(),
        ));
    }
    let mek_generation = info.mek_generation;

    deps.ensure_channel_segment_record(community_id, channel_id)
        .await?;
    deps.require_channel_permission(
        community_id,
        Some(channel_id),
        Permissions::SEND_MESSAGES.bits(),
    )?;
    enforce_slowmode_with_bypass(deps, community_id, channel_id, timestamp_ms)?;

    let sender_key = deps
        .my_pseudonym_hex(community_id)
        .or_else(|| deps.owner_key())
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let lamport_ts = deps.increment_lamport(community_id);
    let context = deps.channel_write_context(community_id, channel_id)?;
    let mek = deps
        .community_mek(community_id)
        .ok_or_else(|| ChannelError::MekMissing {
            community: community_id.into(),
            channel: channel_id.into(),
        })?;
    let ciphertext = encrypt_channel_body(
        &mek,
        &context.channel_key,
        context.slot_index,
        lamport_ts,
        body.as_bytes(),
    )?;
    let message_id = random_message_id("msg_");
    let sequence = deps.next_channel_sequence(community_id, channel_id);

    let outcome = ChannelSendOutcome {
        message_id: message_id.clone(),
        sender_pseudonym_hex: sender_key.clone(),
        ciphertext: ciphertext.clone(),
        mek_generation,
        lamport_ts,
        timestamp_ms,
    };
    deps.persist_sent_message(community_id, channel_id, &outcome, body)
        .await?;
    deps.persist_channel_sequence(community_id, channel_id, sequence)
        .await?;
    deps.mark_last_send_at(community_id, channel_id, timestamp_ms);
    deps.persist_slowmode_state(community_id, channel_id, timestamp_ms)
        .await?;

    let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
        resolve_outbound_mentions(deps, community_id, &sender_key, body);

    let channel_msg = build_channel_message(
        sequence,
        sender_key.clone(),
        ciphertext.clone(),
        mek_generation,
        timestamp_ms,
        lamport_ts,
        message_id.clone(),
        mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    );

    let status = match deps
        .write_channel_message_smpl(&context, &channel_msg)
        .await
    {
        Ok(()) => {
            let notification =
                build_message_notification(channel_id, &channel_msg, context.slot_index)?;
            // gossip is best-effort — the SMPL write is authoritative
            let _ = deps.send_to_mesh(community_id, &notification);
            "delivered".to_string()
        }
        Err(error) => {
            tracing::warn!(error = %error, "channel delivery failed; queueing retry");
            let bytes = serde_json::to_vec(&channel_msg).map_err(|e| {
                ChannelError::Encoding(format!("serialize retry write: {e}"))
            })?;
            deps.enqueue_channel_retry(PendingChannelWrite {
                record_key: context.channel_key.clone(),
                subkey: channel_message_subkey(context.slot_index),
                data: bytes,
            })
            .await?;
            "queued".to_string()
        }
    };

    let timestamp_u64 = u64::try_from(timestamp_ms).unwrap_or_default();
    let echo = SentChannelMessageEcho {
        message_id: message_id.clone(),
        sender_pseudonym: sender_key.clone(),
        timestamp_ms: timestamp_u64,
        body: body.to_string(),
        channel_id: channel_id.to_string(),
    };
    deps.emit_chat_event_local(&echo);

    Ok(ChannelSendResult {
        status,
        message_id,
        sender_pseudonym: sender_key,
        timestamp_ms: timestamp_u64,
        body: body.to_string(),
    })
}

/// Phase 19.g — full forward_channel_message pipeline.
///
/// Forwards a previously-cached source message into a destination
/// channel. Cross-community-safe because pseudonyms aren't linkable
/// across community-scoped derivations (architecture §6.5).
#[allow(clippy::too_many_arguments, reason = "Mirrors src-tauri forward_message signature; src vs dest community/channel/message ids are all required for cache lookup + dest write.")]
pub async fn forward_channel_message<D: ChannelMessagingDeps>(
    deps: &D,
    _source_community_id: &str,
    source_channel_id: &str,
    source_message_id: &str,
    dest_community_id: &str,
    dest_channel_id: &str,
) -> Result<ChannelSendResult, ChannelError> {
    deps.require_channel_permission(
        dest_community_id,
        Some(dest_channel_id),
        Permissions::SEND_MESSAGES.bits(),
    )?;

    let dest_info = deps
        .channel_info(dest_community_id, dest_channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(dest_channel_id.into()))?;
    if dest_info.is_forum {
        return Err(ChannelError::Adapter(
            "forum channels accept posts only through thread creation".into(),
        ));
    }

    let timestamp_ms = i64::try_from(rekindle_utils::timestamp_secs() * 1000).unwrap_or(i64::MAX);
    enforce_slowmode_with_bypass(deps, dest_community_id, dest_channel_id, timestamp_ms)?;

    let source = deps
        .find_channel_message_by_id(source_channel_id, source_message_id)
        .await
        .ok_or_else(|| ChannelError::Adapter("source message not in local cache".into()))?;

    let dest_mek_generation = dest_info.mek_generation;
    let forwarder_pseudonym = deps
        .my_pseudonym_hex(dest_community_id)
        .or_else(|| deps.owner_key())
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(dest_community_id.into()))?;
    let new_message_id = random_message_id("msg_");
    let lamport_ts = deps.increment_lamport(dest_community_id);
    let dest_context = deps.channel_write_context(dest_community_id, dest_channel_id)?;
    let mek = deps
        .community_mek(dest_community_id)
        .ok_or_else(|| ChannelError::MekMissing {
            community: dest_community_id.into(),
            channel: dest_channel_id.into(),
        })?;
    let dest_ciphertext = encrypt_channel_body(
        &mek,
        &dest_context.channel_key,
        dest_context.slot_index,
        lamport_ts,
        source.body.as_bytes(),
    )?;
    let sequence = deps.next_channel_sequence(dest_community_id, dest_channel_id);

    let outcome = ChannelSendOutcome {
        message_id: new_message_id.clone(),
        sender_pseudonym_hex: forwarder_pseudonym.clone(),
        ciphertext: dest_ciphertext.clone(),
        mek_generation: dest_mek_generation,
        lamport_ts,
        timestamp_ms,
    };
    deps.persist_forwarded_message(
        dest_community_id,
        dest_channel_id,
        &outcome,
        &source.body,
        &source.sender_key,
    )
    .await?;
    deps.persist_channel_sequence(dest_community_id, dest_channel_id, sequence)
        .await?;

    let forward_payload = ChannelForward {
        sequence,
        sender_pseudonym: forwarder_pseudonym.clone(),
        original_message_id: source_message_id.to_string(),
        original_channel_id: source_channel_id.to_string(),
        original_author: source.sender_key.clone(),
        content_snapshot: dest_ciphertext.clone(),
        mek_generation: dest_mek_generation,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        lamport_ts,
        message_id: Some(new_message_id.clone()),
    };

    let status = match deps
        .write_channel_forward_smpl(&dest_context, &forward_payload)
        .await
    {
        Ok(()) => {
            let notification =
                build_forward_notification(dest_channel_id, &forward_payload, dest_context.slot_index)?;
            let _ = deps.send_to_mesh(dest_community_id, &notification);
            deps.mark_last_send_at(dest_community_id, dest_channel_id, timestamp_ms);
            let _ = deps
                .persist_slowmode_state(dest_community_id, dest_channel_id, timestamp_ms)
                .await;
            "delivered".to_string()
        }
        Err(error) => {
            tracing::warn!(error = %error, "channel forward write failed");
            "failed".to_string()
        }
    };

    Ok(ChannelSendResult {
        status,
        message_id: new_message_id,
        sender_pseudonym: forwarder_pseudonym,
        timestamp_ms: u64::try_from(timestamp_ms).unwrap_or_default(),
        body: source.body,
    })
}

/// Process a single retry attempt from the channel write queue. Used
/// by the src-tauri retry-loop worker — it pops a `PendingChannelWrite`
/// from the receiver and calls this for each.
///
/// Adapter is responsible for backoff between attempts; the crate
/// only carries the protocol step.
pub async fn process_retry_write<D: ChannelMessagingDeps>(
    deps: &D,
    pending: &PendingChannelWrite,
    context: &ChannelWriteContext,
) -> Result<(), ChannelError> {
    let message: ChannelMessage = serde_json::from_slice(&pending.data).map_err(|e| {
        ChannelError::Encoding(format!("deserialize queued channel message: {e}"))
    })?;
    deps.write_channel_message_smpl(context, &message).await?;
    let notification = build_message_notification(&context.channel_id, &message, context.slot_index)?;
    let _ = deps.send_to_mesh(&context.community_id, &notification);
    if let Some(msg_id) = message.message_id {
        deps.emit_delivery_succeeded(&context.community_id, &context.channel_id, &msg_id);
    }
    Ok(())
}
