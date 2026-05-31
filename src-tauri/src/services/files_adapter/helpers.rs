//! Phase 15.r split — extracted bodies for `FilesAdapter` trait
//! methods that would otherwise blow the per-file LoC cap. Each
//! helper is a free fn taking explicit references so the trait
//! method bodies stay short delegations.

use std::path::Path;
use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_files::{FilesError, FilesEvent};
use rekindle_protocol::dht::community::channel_record::{
    write_member_attachment_cached, write_member_message, ChannelAttachmentCached, ChannelMessage,
};
use rekindle_protocol::dht::DHTManager;

use crate::channels::CommunityEvent;
use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

/// Shared prep for `write_channel_message_to_smpl` and
/// `write_attachment_cached_to_smpl`: parse the slot keypair string,
/// fetch a `RoutingContext`, construct a `DHTManager`, and look up
/// `(pseudonym, signing_key)` for the community.
pub(super) struct DhtWriterContext {
    pub(super) writer: veilid_core::KeyPair,
    pub(super) mgr: DHTManager,
    pub(super) author_pseudo: rekindle_types::id::PseudonymKey,
    pub(super) signing_key: ed25519_dalek::SigningKey,
}

pub(super) fn build_dht_writer_context(
    state: &Arc<AppState>,
    community_id: &str,
    slot_keypair: &str,
) -> Result<DhtWriterContext, FilesError> {
    let writer = slot_keypair
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| FilesError::Transport(format!("invalid slot keypair: {e}")))?;
    let rc = state_helpers::safe_routing_context(state)
        .ok_or_else(|| FilesError::Transport("not attached".into()))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) = state_helpers::pseudonym_credentials(state, community_id)
        .map_err(FilesError::Transport)?;
    Ok(DhtWriterContext {
        writer,
        mgr,
        author_pseudo,
        signing_key,
    })
}

pub(super) async fn write_channel_message_impl(
    state: &Arc<AppState>,
    community_id: &str,
    channel_log_key: &str,
    slot_index: u32,
    slot_keypair: &str,
    message: &ChannelMessage,
) -> Result<(), FilesError> {
    let ctx = build_dht_writer_context(state, community_id, slot_keypair)?;
    write_member_message(
        &ctx.mgr,
        channel_log_key,
        slot_index,
        ctx.writer,
        ctx.author_pseudo,
        &ctx.signing_key,
        message,
    )
    .await
    .map_err(|e| FilesError::Transport(format!("SMPL channel write: {e}")))
}

pub(super) async fn write_attachment_cached_impl(
    state: &Arc<AppState>,
    community_id: &str,
    channel_log_key: &str,
    slot_index: u32,
    slot_keypair: &str,
    cached: &ChannelAttachmentCached,
) -> Result<(), FilesError> {
    let ctx = build_dht_writer_context(state, community_id, slot_keypair)?;
    write_member_attachment_cached(
        &ctx.mgr,
        channel_log_key,
        slot_index,
        ctx.writer,
        ctx.author_pseudo,
        &ctx.signing_key,
        cached,
    )
    .await
    .map_err(|e| FilesError::Transport(format!("AttachmentCached SMPL write: {e}")))
}

/// 3-tier MEK cascade matching the pre-Phase-15 `unwrap_fek_for_offer`
/// body: keystore (historical) → channel_mek_cache → mek_cache.
pub(super) fn historical_channel_mek_impl(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Option<MediaEncryptionKey> {
    // Tier 1: keystore lookup at the requested generation.
    let keystore = &state.keystore;
    let guard = keystore.lock();
    let from_keystore = guard.as_ref().and_then(|ks| {
        crate::keystore::load_channel_mek_generation(ks, community_id, channel_id, generation)
    });
    drop(guard);
    if let Some(mek) = from_keystore {
        return Some(mek);
    }

    // Tier 2: per-channel cache, only if it matches the generation.
    let cache = state.channel_mek_cache.lock();
    if let Some(mek) = cache.get(&(community_id.to_string(), channel_id.to_string())) {
        if mek.generation() == generation {
            return Some(mek.clone());
        }
    }
    drop(cache);

    // Tier 3: community MEK at the requested generation.
    state
        .mek_cache
        .lock()
        .get(community_id)
        .filter(|m| m.generation() == generation)
        .cloned()
}

#[allow(clippy::too_many_arguments, reason = "mirrors message_repo::insert_channel_message_full's SQL column shape — see deps.rs for the rationale")]
pub(super) async fn insert_channel_message_full_impl(
    pool: &DbPool,
    owner_key: &str,
    channel_id: &str,
    sender_key: &str,
    message_id: &str,
    timestamp_ms: i64,
    mek_generation: u64,
    lamport_ts: u64,
    attachment_json: &str,
    flags: u32,
    body: &str,
) -> Result<(), FilesError> {
    let mek_generation = i64::try_from(mek_generation).unwrap_or(i64::MAX);
    let owner = owner_key.to_string();
    let chan = channel_id.to_string();
    let sender = sender_key.to_string();
    let mid = message_id.to_string();
    let attachment_json = attachment_json.to_string();
    let body = body.to_string();
    crate::db_helpers::db_call(pool, move |conn| {
        crate::message_repo::insert_channel_message_full(
            conn,
            &owner,
            &chan,
            &sender,
            &body,
            timestamp_ms,
            true,
            Some(mek_generation),
            &mid,
            lamport_ts,
            false,
            None,
            flags,
            Some(&attachment_json),
        )
    })
    .await
    .map_err(|e| FilesError::Db(format!("insert attachment row: {e}")))
}

pub(super) async fn persist_local_path_impl(
    pool: &DbPool,
    owner_key: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    save_path: &Path,
) -> Result<(), FilesError> {
    let owner = owner_key.to_string();
    let chan = channel_id.to_string();
    let attachment_id_hex = attachment_id_hex.to_string();
    let new_path = save_path.display().to_string();
    crate::db_helpers::db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, attachment_json FROM messages \
             WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
             AND attachment_json IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner, chan], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        for (message_id, json) in rows {
            let Ok(mut record) =
                serde_json::from_str::<rekindle_files::AttachmentRecordJson>(&json)
            else {
                continue;
            };
            if record.attachment_id != attachment_id_hex {
                continue;
            }
            record.local_path = Some(new_path.clone());
            let updated = serde_json::to_string(&record).unwrap_or_else(|_| json.clone());
            conn.execute(
                "UPDATE messages SET attachment_json = ?1 \
                 WHERE owner_key = ?2 AND conversation_id = ?3 AND message_id = ?4",
                rusqlite::params![updated, owner, chan, message_id],
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|e| FilesError::Db(format!("update attachment_json local_path: {e}")))
}

pub(super) fn persist_slowmode_state_impl(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) {
    let owner_for_db = state_helpers::owner_key_or_default(state);
    if owner_for_db.is_empty() {
        return;
    }
    let community_for_db = community_id.to_string();
    let channel_for_db = channel_id.to_string();
    crate::db_helpers::db_fire(
        pool,
        "persist channel_slowmode_state (files_adapter)",
        move |conn| {
            conn.execute(
                "INSERT INTO channel_slowmode_state \
                 (owner_key, community_id, channel_id, last_send_ms) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
                   last_send_ms = excluded.last_send_ms",
                rusqlite::params![owner_for_db, community_for_db, channel_for_db, now_ms],
            )?;
            Ok(())
        },
    );
    // Mirror the in-memory channel_last_send_at update too.
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        cs.channel_last_send_at
            .insert(channel_id.to_string(), now_ms);
    }
}

/// Pure `FilesEvent → CommunityEvent` mapping.
pub(super) fn map_files_event(event: FilesEvent) -> CommunityEvent {
    match event {
        FilesEvent::AttachmentDownloaded {
            community_id,
            channel_id,
            attachment_id_hex,
            local_path,
        } => CommunityEvent::AttachmentDownloaded {
            community_id,
            channel_id,
            attachment_id: attachment_id_hex,
            local_path,
        },
        FilesEvent::ExpressionAssetReady {
            community_id,
            expression_id_hex,
        } => CommunityEvent::ExpressionAssetReady {
            community_id,
            expression_id: expression_id_hex,
        },
    }
}
