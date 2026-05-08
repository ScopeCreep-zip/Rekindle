use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::{ChannelForward, ChannelMessage};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_records::retry::{self, PendingWrite};
use tauri::Emitter;

use crate::channels::{ChatEvent, CommunityEvent};
use crate::db::DbPool;
use crate::db_helpers::{db_call, db_fire};
use crate::state::{AppState, SharedState};
use crate::state_helpers;

/// Pending channel message payload stored in the generic retry queue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChannelMessage {
    pub community_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub author_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: i64,
    pub subkey_index: u32,
    pub lamport_ts: u64,
    pub sequence: u64,
    pub content_hash: String,
    /// Architecture §28.5 — cleartext mention metadata that must
    /// survive the queue-retry round trip. Empty for messages enqueued
    /// before the mention fields existed (legacy rows decode with
    /// these defaulted via serde). `mention_flag_bits` is the OR of
    /// `MENTION_EVERYONE` / `MENTION_HERE` bits per
    /// `rekindle_types::channel::flags`.
    #[serde(default)]
    pub mentioned_pseudonyms: Vec<String>,
    #[serde(default)]
    pub mentioned_roles: Vec<String>,
    #[serde(default)]
    pub mention_flag_bits: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResult {
    pub status: String,
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct SentChannelMessage {
    pub result: SendChannelMessageResult,
    pub sender_key: String,
    pub timestamp: u64,
    pub body: String,
}

struct ChannelWriteContext {
    community_id: String,
    channel_id: String,
    /// SMPL record key for the channel record this writer targets.
    /// For segment-0 senders: the genesis channel record from
    /// `community.channel_log_keys`. For segment-N (N>0) senders: the
    /// per-segment record announced via `ChannelSegmentLinked` (lazily
    /// created on first segment-N write per architecture §15.4).
    channel_key: String,
    slot_keypair: String,
    slot_index: u32,
    /// Which segment hosts the writer. Captured here so retry queues +
    /// audit logging carry segment context.
    segment_index: u32,
}

pub fn start_write_retry_worker(state: Arc<AppState>) {
    let (handle, mut rx) = retry::create_write_queue(128);
    *state.channel_write_retry_tx.write() = Some(handle);

    tauri::async_runtime::spawn(async move {
        while let Some(pending) = rx.recv().await {
            process_retry_write(&state, pending).await;
        }
    });
}

/// Architecture §28.7 + §32 Week 18 slowmode gate. Returns `Ok` when
/// the send is allowed; returns a "slowmode active" error including
/// the remaining seconds when not. `BYPASS_SLOWMODE` (1<<29) lets the
/// caller skip the gate entirely.
pub(crate) fn enforce_slowmode(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    use rekindle_governance::permissions::compute_permissions;
    use rekindle_types::id::PseudonymKey;
    use rekindle_types::permissions::BYPASS_SLOWMODE;

    let (slowmode_seconds, last_send_ms, my_pseudonym, governance) = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return Ok(());
        };
        let slowmode = community
            .channels
            .iter()
            .find(|ch| ch.id == channel_id)
            .and_then(|ch| ch.slowmode_seconds)
            .filter(|&secs| secs > 0);
        let last = community
            .channel_last_send_at
            .get(channel_id)
            .copied()
            .unwrap_or(0);
        (
            slowmode,
            last,
            community.my_pseudonym_key.clone(),
            community.governance_state.clone(),
        )
    };

    let Some(slowmode_seconds) = slowmode_seconds else {
        return Ok(());
    };

    // Bypass check.
    if let (Some(pk_hex), Some(governance)) = (my_pseudonym, governance) {
        if let Ok(pk_bytes) = hex::decode(&pk_hex) {
            if let Ok(pk_arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) {
                let perms = compute_permissions(
                    &PseudonymKey(pk_arr),
                    None,
                    &governance,
                    rekindle_utils::timestamp_secs(),
                );
                if perms & BYPASS_SLOWMODE == BYPASS_SLOWMODE {
                    return Ok(());
                }
            }
        }
    }

    let elapsed_ms = now_ms.saturating_sub(last_send_ms);
    let required_ms = i64::from(slowmode_seconds).saturating_mul(1000);
    if elapsed_ms < required_ms {
        let remaining_secs = (required_ms - elapsed_ms + 999) / 1000;
        return Err(format!(
            "slowmode active — wait {remaining_secs}s before sending again"
        ));
    }
    Ok(())
}

pub async fn send_message(
    state: &SharedState,
    pool: &DbPool,
    channel_id: &str,
    body: &str,
) -> Result<SentChannelMessage, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let timestamp_ms = crate::db::timestamp_now();

    let (community_id, mek_generation) = {
        let communities = state.communities.read();
        let community = communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .ok_or("channel not found in any community")?;
        if community
            .channels
            .iter()
            .any(|channel| {
                channel.id == channel_id
                    && matches!(channel.channel_type, crate::state::ChannelType::Forum)
            })
        {
            return Err("forum channels accept posts only through thread creation".to_string());
        }
        (community.id.clone(), community.mek_generation)
    };

    // Plate Gate (architecture §15.4): if I'm in segment N>0, ensure a
    // channel-segment SMPL record exists for me to write to — lazy
    // creation announces a `ChannelSegmentLinked` governance entry on
    // first segment-N write per channel. No-op for segment 0 (genesis
    // record always exists).
    crate::services::community::segments::ensure_channel_segment_record(
        state,
        &community_id,
        channel_id,
    )
    .await?;

    crate::commands::community::require_permission(
        state,
        &community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::SEND_MESSAGES,
    )?;

    // Architecture §28.7 slowmode: if the channel has a non-zero
    // `slowmode_seconds` and we wrote within that window without
    // BYPASS_SLOWMODE, reject the send. The frontend already shows a
    // countdown via `MessageInput.tsx`, but the backend gate is the
    // authoritative one — it survives a malicious or out-of-date UI.
    enforce_slowmode(state, &community_id, channel_id, timestamp_ms)?;

    let sender_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_else(|| owner_key.clone())
    };

    // Architecture §8 line 1626: bind the ciphertext to its channel +
    // writer slot + Lamport position via AES-GCM AAD. We must compute
    // lamport_ts and look up the SMPL channel-record context BEFORE
    // encrypting so the AAD is available.
    let lamport_ts = state_helpers::increment_lamport(state, &community_id);
    let context = channel_write_context(state, &community_id, channel_id)?;
    let aad = rekindle_crypto::group::media_key::ChannelAad {
        channel_record_key: context.channel_key.as_bytes(),
        subkey_index: context.slot_index,
        lamport_ts,
    };

    let ciphertext = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(&community_id).ok_or_else(|| {
            "MEK not available — rejoin the community or wait for MEK delivery".to_string()
        })?;
        mek.encrypt_with_aad(body.as_bytes(), aad)
            .map_err(|e| format!("MEK encryption failed: {e}"))?
    };

    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let sequence = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            let s = cs
                .channel_sequences
                .entry(channel_id.to_string())
                .or_insert(0);
            *s += 1;
            *s
        } else {
            1
        }
    };

    let channel_id_owned = channel_id.to_string();
    let sender_key_owned = sender_key.clone();
    let body_owned = body.to_string();
    let owner_key_owned = owner_key.clone();
    let message_id_for_db = message_id.clone();
    db_call(pool, move |conn| {
        crate::message_repo::insert_channel_message_with_protocol_metadata(
            conn,
            &owner_key_owned,
            &channel_id_owned,
            &sender_key_owned,
            &body_owned,
            timestamp_ms,
            true,
            Some(i64::try_from(mek_generation).unwrap_or(i64::MAX)),
            &message_id_for_db,
            lamport_ts,
            false,
        )
    })
    .await?;

    persist_channel_sequence(pool, &owner_key, &community_id, channel_id, sequence);

    // Architecture §28.7 slowmode bookkeeping: record the local
    // timestamp of this send so the next call to `enforce_slowmode`
    // can compute the elapsed window. Architecture §32 W18 — also
    // persisted to SQLite so the slowmode window survives restarts;
    // otherwise a user could quit the app to bypass the cooldown.
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            cs.channel_last_send_at
                .insert(channel_id.to_string(), timestamp_ms);
        }
    }
    {
        let owner_for_db = owner_key.clone();
        let community_for_db = community_id.clone();
        let channel_for_db = channel_id.to_string();
        crate::db_helpers::db_fire(pool, "persist channel_slowmode_state", move |conn| {
            conn.execute(
                "INSERT INTO channel_slowmode_state \
                 (owner_key, community_id, channel_id, last_send_ms) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
                   last_send_ms = excluded.last_send_ms",
                rusqlite::params![owner_for_db, community_for_db, channel_for_db, timestamp_ms],
            )?;
            Ok(())
        });
    }

    // Architecture §28.5 line 3105-3120 — resolve mentions BEFORE
    // encryption and stamp them on the cleartext envelope. Receivers
    // route notifications without decrypting the body. The
    // `@everyone` / `@here` signals piggyback on the existing `flags`
    // u32 so non-mention messages stay byte-for-byte identical to
    // the legacy wire shape.
    let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
        resolve_outbound_mentions(state, &community_id, &sender_key, body);

    let channel_msg = ChannelMessage {
        sequence,
        sender_pseudonym: sender_key.clone(),
        ciphertext: ciphertext.clone(),
        mek_generation,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        reply_to: None,
        lamport_ts,
        message_id: Some(message_id.clone()),
        attachment: None,
        flags: mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    };

    let status = match write_message_once(state, &context, &channel_msg, context.slot_index).await {
        Ok(()) => "delivered".to_string(),
        Err(error) => {
            tracing::warn!(error = %error, "channel delivery failed; queueing retry");
            enqueue_retry(
                state,
                &context.channel_key,
                crate::services::community::channel_message_subkey(context.slot_index),
                &channel_msg,
            )
            .await?;
            "queued".to_string()
        }
    };

    Ok(SentChannelMessage {
        result: SendChannelMessageResult { status, message_id },
        sender_key,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        body: body.to_string(),
    })
}

/// Forward a previously-cached channel message to a destination channel.
///
/// The source must already be in local SQLite (forwarders never refetch from DHT —
/// the original community's MEK is not available to them on cross-community
/// forwards). The body is re-encrypted with the destination community's MEK and
/// written as a `ChannelEntry::Forward` (`ChannelRecordEntry::Forward` on the
/// wire), which destination peers render with a "Forwarded from" attribution.
///
/// Privacy: `original_author` is preserved for display, but pseudonyms in different
/// communities use independent derivations — the value is NOT linkable across
/// communities and reveals nothing about the source community's identity namespace.
pub async fn forward_message(
    state: &SharedState,
    pool: &DbPool,
    _source_community_id: &str,
    source_channel_id: &str,
    source_message_id: &str,
    dest_community_id: &str,
    dest_channel_id: &str,
) -> Result<SentChannelMessage, String> {
    crate::commands::community::require_permission(
        state,
        dest_community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::SEND_MESSAGES,
    )?;
    {
        let communities = state.communities.read();
        let dest = communities
            .get(dest_community_id)
            .ok_or("destination community not found")?;
        if dest.channels.iter().any(|ch| {
            ch.id == dest_channel_id
                && matches!(ch.channel_type, crate::state::ChannelType::Forum)
        }) {
            return Err(
                "forum channels accept posts only through thread creation".to_string(),
            );
        }
    }

    // Architecture §28.7 — forwards are new writes to the destination
    // channel, so the slowmode gate applies there. Without this, a user
    // throttled by send_message could spam by repeatedly forwarding.
    enforce_slowmode(
        state,
        dest_community_id,
        dest_channel_id,
        crate::db::timestamp_now(),
    )?;

    let owner_key = state_helpers::current_owner_key(state)?;

    let source_channel_id_owned = source_channel_id.to_string();
    let source_message_id_owned = source_message_id.to_string();
    let owner_key_for_lookup = owner_key.clone();
    let source = db_call(pool, move |conn| {
        crate::message_repo::find_channel_message_by_id(
            conn,
            &owner_key_for_lookup,
            &source_channel_id_owned,
            &source_message_id_owned,
        )
    })
    .await?
    .ok_or("source message not in local cache")?;

    let dest_mek_generation = {
        let communities = state.communities.read();
        let dest = communities
            .get(dest_community_id)
            .ok_or("destination community not found")?;
        dest.mek_generation
    };
    let forwarder_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(dest_community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_else(|| owner_key.clone())
    };

    // Architecture §8 line 1626 — AAD must be computed from the
    // destination channel's record key + slot + lamport, not the
    // source's. Compute lamport + context for dest first, then
    // encrypt with the matching AAD.
    let timestamp_ms = crate::db::timestamp_now();
    let new_message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let lamport_ts = state_helpers::increment_lamport(state, dest_community_id);
    let dest_context = channel_write_context(state, dest_community_id, dest_channel_id)?;
    let aad = rekindle_crypto::group::media_key::ChannelAad {
        channel_record_key: dest_context.channel_key.as_bytes(),
        subkey_index: dest_context.slot_index,
        lamport_ts,
    };
    let dest_ciphertext = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(dest_community_id).ok_or_else(|| {
            "destination MEK not available — rejoin the community or wait for MEK delivery"
                .to_string()
        })?;
        mek.encrypt_with_aad(source.body.as_bytes(), aad)
            .map_err(|e| format!("dest MEK encryption failed: {e}"))?
    };
    let sequence = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(dest_community_id) {
            let s = cs
                .channel_sequences
                .entry(dest_channel_id.to_string())
                .or_insert(0);
            *s += 1;
            *s
        } else {
            1
        }
    };

    let dest_channel_id_owned = dest_channel_id.to_string();
    let owner_key_for_insert = owner_key.clone();
    let forwarder_for_insert = forwarder_pseudonym.clone();
    let body_for_insert = source.body.clone();
    let new_message_id_for_insert = new_message_id.clone();
    let original_author_for_insert = source.sender_key.clone();
    db_call(pool, move |conn| {
        crate::message_repo::insert_channel_message_with_full_metadata(
            conn,
            &owner_key_for_insert,
            &dest_channel_id_owned,
            &forwarder_for_insert,
            &body_for_insert,
            timestamp_ms,
            true,
            Some(i64::try_from(dest_mek_generation).unwrap_or(i64::MAX)),
            &new_message_id_for_insert,
            lamport_ts,
            false,
            Some(&original_author_for_insert),
        )
    })
    .await?;

    let context = channel_write_context(state, dest_community_id, dest_channel_id)?;
    persist_channel_sequence(pool, &owner_key, dest_community_id, dest_channel_id, sequence);

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

    let status = match write_forward_once(state, &context, &forward_payload).await {
        Ok(()) => "delivered".to_string(),
        Err(error) => {
            tracing::warn!(error = %error, "channel forward write failed");
            "failed".to_string()
        }
    };

    // Architecture §28.7 — record this forward as a slowmode-eligible
    // send so a follow-up text/file/voice in the same channel respects
    // the cooldown.
    if status == "delivered" {
        let now_ms = crate::db::timestamp_now();
        {
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(dest_community_id) {
                cs.channel_last_send_at
                    .insert(dest_channel_id.to_string(), now_ms);
            }
        }
        let owner_for_db = owner_key.clone();
        let community_for_db = dest_community_id.to_string();
        let channel_for_db = dest_channel_id.to_string();
        crate::db_helpers::db_fire(pool, "persist channel_slowmode_state (forward)", move |conn| {
            conn.execute(
                "INSERT INTO channel_slowmode_state \
                 (owner_key, community_id, channel_id, last_send_ms) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
                   last_send_ms = excluded.last_send_ms",
                rusqlite::params![owner_for_db, community_for_db, channel_for_db, now_ms],
            )?;
            Ok(())
        });
    }

    Ok(SentChannelMessage {
        result: SendChannelMessageResult {
            status,
            message_id: new_message_id,
        },
        sender_key: forwarder_pseudonym,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        body: source.body,
    })
}

async fn write_forward_once(
    state: &SharedState,
    context: &ChannelWriteContext,
    forward: &ChannelForward,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let writer = context
        .slot_keypair
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, &context.community_id)?;
    rekindle_protocol::dht::community::channel_record::write_member_forward(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        forward,
    )
    .await
    .map_err(|e| format!("SMPL channel forward write failed: {e}"))?;

    let notification = CommunityEnvelope::MessageNotification {
        channel_id: context.channel_id.clone(),
        message_id: forward
            .message_id
            .clone()
            .ok_or("forward missing message_id")?,
        author_pseudonym: forward.sender_pseudonym.clone(),
        subkey_index: crate::services::community::channel_message_subkey(context.slot_index),
        lamport_ts: forward.lamport_ts,
        sequence: forward.sequence,
        content_hash: blake3::hash(&forward.content_snapshot).to_hex().to_string(),
        timestamp: forward.timestamp,
    };
    crate::services::community::send_to_mesh(state, &context.community_id, &notification)?;
    Ok(())
}

async fn enqueue_retry(
    state: &SharedState,
    record_key: &str,
    subkey: u32,
    message: &ChannelMessage,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(message).map_err(|e| format!("serialize retry write: {e}"))?;
    let handle = state
        .channel_write_retry_tx
        .read()
        .clone()
        .ok_or("channel write retry queue unavailable")?;
    handle.enqueue(record_key.to_string(), subkey, bytes).await;
    Ok(())
}

async fn process_retry_write(state: &SharedState, pending: PendingWrite) {
    let mut current = pending;
    loop {
        match retry_write_once(state, &current).await {
            Ok(()) => return,
            Err(error) => {
                tracing::debug!(
                    record_key = %current.record_key,
                    subkey = current.subkey,
                    attempt = current.attempt,
                    error = %error,
                    "channel write retry attempt failed"
                );
                if let Some(next) = retry::retry_or_fail(&current) {
                    tokio::time::sleep(retry::backoff_duration(current.attempt)).await;
                    current = next;
                    continue;
                }
                emit_delivery_failed(state, &current, &error);
                return;
            }
        }
    }
}

async fn retry_write_once(state: &SharedState, pending: &PendingWrite) -> Result<(), String> {
    let context = find_context_by_record_key(state, &pending.record_key)
        .ok_or("channel write retry context not found")?;
    let message: ChannelMessage = serde_json::from_slice(&pending.data)
        .map_err(|e| format!("deserialize queued channel message: {e}"))?;
    // Trace the segment context BEFORE the actual write attempt so any
    // backend tracing pipeline correlating retry-failures to a segment
    // (Plate Gate audit case) sees the join even if `write_message_once`
    // succeeds. Mirrors the contract on `ChannelWriteContext.segment_index`.
    tracing::trace!(
        community = %context.community_id,
        channel = %context.channel_id,
        segment_index = context.segment_index,
        attempt = pending.attempt,
        "retrying queued channel write"
    );
    write_message_once(state, &context, &message, context.slot_index).await?;
    emit_delivery_event(
        state,
        CommunityEvent::ChannelMessageDelivered {
            community_id: context.community_id,
            channel_id: context.channel_id,
            message_id: message
                .message_id
                .ok_or("queued channel message missing message_id")?,
        },
    );
    Ok(())
}

async fn write_message_once(
    state: &SharedState,
    context: &ChannelWriteContext,
    channel_msg: &ChannelMessage,
    slot_index: u32,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let writer = context
        .slot_keypair
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, &context.community_id)?;
    rekindle_protocol::dht::community::channel_record::write_member_message(
        &mgr,
        &context.channel_key,
        slot_index,
        writer,
        author_pseudo,
        &signing_key,
        channel_msg,
    )
    .await
    .map_err(|e| format!("SMPL channel write failed: {e}"))?;

    let notification = build_message_notification(&context.channel_id, channel_msg, slot_index)?;
    crate::services::community::send_to_mesh(state, &context.community_id, &notification)?;
    Ok(())
}

fn build_message_notification(
    channel_id: &str,
    channel_msg: &ChannelMessage,
    slot_index: u32,
) -> Result<CommunityEnvelope, String> {
    Ok(CommunityEnvelope::MessageNotification {
        channel_id: channel_id.to_string(),
        message_id: channel_msg
            .message_id
            .clone()
            .ok_or("channel message missing message_id")?,
        author_pseudonym: channel_msg.sender_pseudonym.clone(),
        subkey_index: crate::services::community::channel_message_subkey(slot_index),
        lamport_ts: channel_msg.lamport_ts,
        sequence: channel_msg.sequence,
        content_hash: blake3::hash(&channel_msg.ciphertext).to_hex().to_string(),
        timestamp: channel_msg.timestamp,
    })
}

fn channel_write_context(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<ChannelWriteContext, String> {
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or("community not found for channel send")?;
    let segment_index = community.my_segment_index.unwrap_or(0);
    let channel_id_bytes: [u8; 16] = hex::decode(channel_id)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 16]);
    let channel_id_typed = rekindle_types::id::ChannelId(channel_id_bytes);

    // Plate Gate (architecture §15.4): for segment-0 senders the channel
    // record is the genesis `channel_log_keys[channel_id]`; for segment-N
    // senders it's the per-segment record announced via
    // `ChannelSegmentLinked`. If no segment-N record exists yet, the
    // caller (`send_message`) must lazily create one before this lookup.
    let channel_key = if segment_index == 0 {
        community
            .channel_log_keys
            .get(channel_id)
            .cloned()
            .ok_or("channel record key missing")?
    } else {
        community
            .governance_state
            .as_ref()
            .and_then(|gov| {
                gov.channel_segment_records
                    .get(&(channel_id_typed, segment_index))
                    .map(|rec| rec.record_key.clone())
            })
            .ok_or_else(|| {
                format!(
                    "no channel record yet for segment {segment_index} of channel {channel_id} — call ensure_channel_segment_record first"
                )
            })?
    };

    Ok(ChannelWriteContext {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        channel_key,
        slot_keypair: community
            .slot_keypair
            .clone()
            .ok_or("slot keypair missing")?,
        slot_index: community
            .my_subkey_index
            .ok_or("community subkey index missing")?,
        segment_index,
    })
}

fn find_context_by_record_key(
    state: &SharedState,
    record_key: &str,
) -> Option<ChannelWriteContext> {
    let communities = state.communities.read();
    for (community_id, community) in &*communities {
        let slot_keypair = community.slot_keypair.clone()?;
        let slot_index = community.my_subkey_index?;
        let segment_index = community.my_segment_index.unwrap_or(0);
        for (channel_id, channel_key) in &community.channel_log_keys {
            if channel_key == record_key {
                return Some(ChannelWriteContext {
                    community_id: community_id.clone(),
                    channel_id: channel_id.clone(),
                    channel_key: channel_key.clone(),
                    slot_keypair,
                    slot_index,
                    segment_index,
                });
            }
        }
    }
    None
}

fn persist_channel_sequence(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    sequence: u64,
) {
    let owner_key = owner_key.to_string();
    let community_id = community_id.to_string();
    let channel_id = channel_id.to_string();
    let sequence = i64::try_from(sequence).unwrap_or(i64::MAX);
    db_fire(pool, "persist channel sequence", move |conn| {
        conn.execute(
            "UPDATE channels SET my_sequence = ?1 WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
            rusqlite::params![sequence, owner_key, community_id, channel_id],
        )?;
        Ok(())
    });
}

fn emit_delivery_failed(state: &SharedState, pending: &PendingWrite, error: &str) {
    let Some(context) = find_context_by_record_key(state, &pending.record_key) else {
        return;
    };
    let Ok(message) = serde_json::from_slice::<ChannelMessage>(&pending.data) else {
        return;
    };
    let Some(message_id) = message.message_id else {
        return;
    };
    tracing::warn!(
        community = %context.community_id,
        channel = %context.channel_id,
        message_id = %message_id,
        error,
        "queued channel write permanently failed"
    );
    emit_delivery_event(
        state,
        CommunityEvent::ChannelMessageDeliveryFailed {
            community_id: context.community_id,
            channel_id: context.channel_id,
            message_id,
        },
    );
}

fn emit_delivery_event(state: &SharedState, event: CommunityEvent) {
    let Some(app_handle) = state_helpers::app_handle(state) else {
        return;
    };
    let _ = app_handle.emit("community-event", event);
}

pub fn emit_local_chat_event(app: &tauri::AppHandle, sent: &SentChannelMessage, channel_id: &str) {
    let event = ChatEvent::MessageReceived {
        from: sent.sender_key.clone(),
        body: sent.body.clone(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: sent.timestamp,
        conversation_id: channel_id.to_string(),
        server_message_id: Some(sent.result.message_id.clone()),
        reply_to_id: None,
        sender_display_name: None,
    };
    let _ = app.emit("chat-event", &event);
}

/// Single source of truth for sender-side mention resolution.
/// Returns `(mentioned_pseudonyms, mentioned_roles, mention_flag_bits)`
/// where `mention_flag_bits` is `MENTION_EVERYONE`, `MENTION_HERE`, or
/// the OR of both (or zero). Reader-validates: `@everyone` / `@here`
/// are stripped if the sender lacks `MENTION_EVERYONE` permission.
pub(crate) fn resolve_outbound_mentions(
    state: &SharedState,
    community_id: &str,
    sender_pseudonym_hex: &str,
    body: &str,
) -> (Vec<String>, Vec<String>, u32) {
    let mut matches =
        crate::services::community::mentions::parse_mentions(state, community_id, body);
    crate::services::community::mentions::validate_sender_permissions(
        state,
        community_id,
        sender_pseudonym_hex,
        &mut matches,
    );
    let (pseudonyms, roles, everyone, here) =
        crate::services::community::mentions::resolve_to_wire(state, community_id, &matches);
    let mut flag_bits = 0u32;
    if everyone {
        flag_bits |= rekindle_types::channel::flags::MENTION_EVERYONE;
    }
    if here {
        flag_bits |= rekindle_types::channel::flags::MENTION_HERE;
    }
    (pseudonyms, roles, flag_bits)
}

#[cfg(test)]
#[path = "channel_messages_tests.rs"]
mod tests;
