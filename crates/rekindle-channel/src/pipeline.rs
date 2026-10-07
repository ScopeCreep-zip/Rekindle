//! Phase 19.g-REDO — channel send/forward orchestrators.
//!
//! Ported from src-tauri/services/community/channel_messages.rs.
//! Crate-side `send_channel_message` and `forward_channel_message`
//! parameterised over `D: ChannelMessagingDeps`. The retry worker
//! itself (loop reading `channel_write_retry_tx`) stays src-tauri
//! because it owns the receiver end of the channel; the crate
//! exposes the per-write retry-enqueue primitive via the deps trait.

use rekindle_codec::community::channel_record::{ChannelForward, ChannelMessage};
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_types::permissions::{BYPASS_SLOWMODE, SEND_MESSAGES};

use crate::deps::{
    ChannelMessagingDeps, ChannelSendOutcome, DhtWrite, PendingDelivery, SentChannelMessageEcho,
};
use crate::error::ChannelError;
use crate::mentions::resolve_outbound_mentions;
use crate::send::{
    build_channel_message, channel_message_subkey, slowmode_check, BodyPosition,
    BuildChannelMessageParams,
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
        content_hash: blake3::hash(&channel_msg.ciphertext).to_hex().to_string(),
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
    format!(
        "{prefix}{}",
        hex::encode(rekindle_utils::random::id_bytes_16())
    )
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
    deps.ensure_channel_segment_record(community_id, channel_id)
        .await?;
    deps.require_channel_permission(community_id, Some(channel_id), SEND_MESSAGES)?;
    enforce_slowmode_with_bypass(deps, community_id, channel_id, timestamp_ms)?;

    let sender_key = deps
        .my_pseudonym_hex(community_id)
        .or_else(|| deps.owner_key())
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let lamport_ts = deps.increment_lamport(community_id)?;
    let context = deps.channel_write_context(community_id, channel_id)?;
    let (ciphertext, mek_generation) = crate::text_keys::seal_text(
        deps,
        community_id,
        channel_id,
        BodyPosition {
            channel_record_key: &context.channel_key,
            subkey_index: channel_message_subkey(context.slot_index),
            lamport_ts,
        },
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

    let channel_msg = build_channel_message(BuildChannelMessageParams {
        sequence,
        sender_pseudonym: sender_key.clone(),
        ciphertext: ciphertext.clone(),
        mek_generation,
        timestamp_ms,
        lamport_ts,
        message_id: message_id.clone(),
        mention_flag_bits: mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    });

    // The SMPL write is authoritative and gossip is the fast path, sent
    // either way: a held write is on its way (plan C7.13).
    let status = match deps
        .write_channel_message_smpl(&context, &channel_msg)
        .await
    {
        Ok(written) => {
            let notification =
                build_message_notification(channel_id, &channel_msg, context.slot_index)?;
            let _ = deps.send_to_mesh(community_id, &notification);
            match written {
                DhtWrite::Stored => "delivered".to_string(),
                DhtWrite::Held => {
                    deps.track_pending_delivery(
                        &context.channel_key,
                        channel_message_subkey(context.slot_index),
                        PendingDelivery {
                            community: community_id.to_string(),
                            channel: channel_id.to_string(),
                            message: message_id.clone(),
                        },
                    );
                    "queued".to_string()
                }
            }
        }
        // Not a network miss (the pool holds those): the write cannot be made.
        Err(error) => {
            tracing::warn!(error = %error, "channel write failed");
            "failed".to_string()
        }
    };

    let timestamp_u64 = u64::try_from(timestamp_ms).unwrap_or_default();
    let echo = SentChannelMessageEcho {
        community_id: community_id.to_string(),
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

/// Source/destination identifiers for a channel forward.
///
/// Borrows every id (`&'a str`) because the orchestrator already holds
/// these as owned strings; forwarding only reads them for cache lookup
/// and the destination write, so no clones are needed.
pub struct ForwardChannelMessageParams<'a> {
    pub source_community: &'a str,
    pub source_channel: &'a str,
    pub source_message: &'a str,
    pub dest_community: &'a str,
    pub dest_channel: &'a str,
}

/// Phase 19.g — full forward_channel_message pipeline.
///
/// Forwards a previously-cached source message into a destination
/// channel. Cross-community-safe because pseudonyms aren't linkable
/// across community-scoped derivations (architecture §6.5).
pub async fn forward_channel_message<D: ChannelMessagingDeps>(
    deps: &D,
    params: ForwardChannelMessageParams<'_>,
) -> Result<ChannelSendResult, ChannelError> {
    let ForwardChannelMessageParams {
        source_community: _source_community_id,
        source_channel: source_channel_id,
        source_message: source_message_id,
        dest_community: dest_community_id,
        dest_channel: dest_channel_id,
    } = params;
    deps.require_channel_permission(dest_community_id, Some(dest_channel_id), SEND_MESSAGES)?;

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

    let forwarder_pseudonym = deps
        .my_pseudonym_hex(dest_community_id)
        .or_else(|| deps.owner_key())
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(dest_community_id.into()))?;
    let new_message_id = random_message_id("msg_");
    let lamport_ts = deps.increment_lamport(dest_community_id)?;
    let dest_context = deps.channel_write_context(dest_community_id, dest_channel_id)?;
    let (dest_ciphertext, dest_mek_generation) = crate::text_keys::seal_text(
        deps,
        dest_community_id,
        dest_channel_id,
        BodyPosition {
            channel_record_key: &dest_context.channel_key,
            subkey_index: channel_message_subkey(dest_context.slot_index),
            lamport_ts,
        },
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
        Ok(written) => {
            let notification = build_forward_notification(
                dest_channel_id,
                &forward_payload,
                dest_context.slot_index,
            )?;
            let _ = deps.send_to_mesh(dest_community_id, &notification);
            deps.mark_last_send_at(dest_community_id, dest_channel_id, timestamp_ms);
            let _ = deps
                .persist_slowmode_state(dest_community_id, dest_channel_id, timestamp_ms)
                .await;
            match written {
                DhtWrite::Stored => "delivered".to_string(),
                DhtWrite::Held => {
                    deps.track_pending_delivery(
                        &dest_context.channel_key,
                        channel_message_subkey(dest_context.slot_index),
                        PendingDelivery {
                            community: dest_community_id.to_string(),
                            channel: dest_channel_id.to_string(),
                            message: new_message_id.clone(),
                        },
                    );
                    "queued".to_string()
                }
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "channel forward write failed");
            "failed".to_string()
        }
    };

    let timestamp_u64 = u64::try_from(timestamp_ms).unwrap_or_default();
    deps.emit_chat_event_local(&SentChannelMessageEcho {
        community_id: dest_community_id.to_string(),
        message_id: new_message_id.clone(),
        sender_pseudonym: forwarder_pseudonym.clone(),
        timestamp_ms: timestamp_u64,
        body: source.body.clone(),
        channel_id: dest_channel_id.to_string(),
    });

    Ok(ChannelSendResult {
        status,
        message_id: new_message_id,
        sender_pseudonym: forwarder_pseudonym,
        timestamp_ms: timestamp_u64,
        body: source.body,
    })
}

/// A held channel write settled (`RecordPool::settled`, plan C7.13): every
/// message waiting on that slot is delivered when it landed. When the
/// network superseded it, our page was replaced by one we did not write and
/// those messages did not land: they fail visibly, and a resend merges.
pub fn on_write_settled<D: ChannelMessagingDeps>(
    deps: &D,
    record_key: &str,
    subkey: u32,
    landed: bool,
) {
    for pending in deps.take_pending_deliveries(record_key, subkey) {
        tracing::info!(
            message = %pending.message,
            landed,
            "held channel write settled: message delivery resolved"
        );
        if landed {
            deps.emit_delivery_succeeded(&pending.community, &pending.channel, &pending.message);
        } else {
            deps.emit_delivery_failed(&pending.community, &pending.channel, &pending.message);
        }
    }
}
