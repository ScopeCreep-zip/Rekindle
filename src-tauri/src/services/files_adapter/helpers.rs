//! Phase 15.r split — extracted bodies for `FilesAdapter` trait
//! methods that would otherwise blow the per-file LoC cap. Each
//! helper is a free fn taking explicit references so the trait
//! method bodies stay short delegations.

use std::path::Path;
use std::sync::Arc;

use rekindle_codec::community::channel_record::{ChannelAttachmentCached, ChannelMessage};
use rekindle_files::{FilesError, FilesEvent};
use rekindle_protocol::dht::community::channel_record::{
    write_member_attachment_cached, write_member_message,
};

use crate::channels::CommunityEvent;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Shared prep for `write_channel_message_to_smpl` and
/// `write_attachment_cached_to_smpl`: parse the slot keypair string,
/// borrow the record from the session's record pool, and look up
/// `(pseudonym, signing_key)` for the community.
pub(super) struct DhtWriterContext {
    pub(super) writer: veilid_core::KeyPair,
    pub(super) pool: std::sync::Arc<rekindle_protocol::dht::pool::RecordPool>,
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
    let pool = state_helpers::record_pool(state).map_err(FilesError::Transport)?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, community_id).map_err(FilesError::Transport)?;
    Ok(DhtWriterContext {
        writer,
        pool,
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
    let written = write_member_message(
        &ctx.pool,
        channel_log_key,
        slot_index,
        ctx.writer,
        ctx.author_pseudo,
        &ctx.signing_key,
        message,
    )
    .await;
    held_is_written(written, "SMPL channel write")
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
    let written = write_member_attachment_cached(
        &ctx.pool,
        channel_log_key,
        slot_index,
        ctx.writer,
        ctx.author_pseudo,
        &ctx.signing_key,
        cached,
    )
    .await;
    held_is_written(written, "AttachmentCached SMPL write")
}

/// A channel write the record pool holds (a miss, or one logout cut short)
/// lands when it can (plan C7.13); file entries carry no delivery status,
/// so held is written.
fn held_is_written(
    written: Result<
        rekindle_protocol::dht::community::channel_record::AppendOutcome,
        rekindle_protocol::ProtocolError,
    >,
    what: &str,
) -> Result<(), FilesError> {
    match written {
        Ok(_) | Err(rekindle_protocol::ProtocolError::PoolClosed) => Ok(()),
        Err(e) => Err(FilesError::Transport(format!("{what}: {e}"))),
    }
}

pub(super) async fn insert_channel_message_full_impl(
    pool: &Db,
    row: rekindle_files::InsertChannelMessage<'_>,
) -> Result<(), FilesError> {
    let mek_generation = i64::try_from(row.mek_generation).unwrap_or(i64::MAX);
    let owner = row.owner_key.to_string();
    let chan = row.channel_id.to_string();
    let sender = row.sender_key.to_string();
    let mid = row.message_id.to_string();
    let attachment_json = row.attachment_json.to_string();
    let body = row.body.to_string();
    let timestamp_ms = row.timestamp_ms;
    let lamport_ts = row.lamport_ts;
    let flags = row.flags;
    crate::db_helpers::db_call(pool, move |conn| {
        crate::message_repo::insert_channel_message_full(
            conn,
            &crate::message_repo::ChannelMessageInsert {
                owner_key: &owner,
                channel_id: &chan,
                sender_key: &sender,
                body: &body,
                timestamp: timestamp_ms,
                is_read: true,
                mek_generation: Some(mek_generation),
                message_id: &mid,
                lamport_ts,
                automod_blurred: false,
                forwarded_from_author: None,
                flags,
                attachment_json: Some(&attachment_json),
            },
        )
    })
    .await
    .map_err(|e| FilesError::Db(format!("insert attachment row: {e}")))
}

pub(super) async fn persist_local_path_impl(
    pool: &Db,
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
    pool: &Db,
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
        } => CommunityEvent::AttachmentDownloaded {
            community_id,
            channel_id,
            attachment_id: attachment_id_hex,
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
