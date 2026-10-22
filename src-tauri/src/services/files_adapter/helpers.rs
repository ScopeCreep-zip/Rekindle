//! Phase 15.r split — extracted bodies for `FilesAdapter` trait
//! methods that would otherwise blow the per-file LoC cap. Each
//! helper is a free fn taking explicit references so the trait
//! method bodies stay short delegations.

use std::path::Path;
use std::sync::Arc;

use rekindle_codec::community::channel_record::{ChannelAttachmentCached, ChannelMessage};
use rekindle_files::{ChannelPost, ChannelWrite, FilesError, FilesEvent};
use rekindle_node::host::adapters::outbox::ChannelPosts;
use rekindle_protocol::dht::community::channel_record::AppendOutcome;
use rekindle_protocol::dht::community::channel_record::{
    write_member_attachment_cached, write_member_message,
};

use crate::channels::CommunityEvent;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Shared prep for `write_channel_message_to_smpl` and
/// `write_attachment_cached_to_smpl`: the session's record pool and the
/// community's `(pseudonym, signing_key)`.
pub(super) struct DhtWriterContext {
    pub(super) pool: Arc<dyn rekindle_records::port::RecordPort>,
    pub(super) author_pseudo: rekindle_types::id::PseudonymKey,
    pub(super) signing_key: ed25519_dalek::SigningKey,
}

pub(super) fn build_dht_writer_context(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<DhtWriterContext, FilesError> {
    let pool = state_helpers::record_pool(state).map_err(FilesError::Transport)?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, community_id).map_err(FilesError::Transport)?;
    Ok(DhtWriterContext {
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
) -> Result<ChannelWrite, FilesError> {
    let ctx = build_dht_writer_context(state, community_id)?;
    let written = write_member_message(
        ctx.pool.as_ref(),
        channel_log_key,
        slot_index,
        slot_keypair,
        ctx.author_pseudo,
        &ctx.signing_key,
        message,
    )
    .await;
    // A held page (or one a logout cut short) is the pool's to land; its
    // settle settles the message's outbox row (plan E2.5).
    match written {
        Ok(AppendOutcome::Stored) => Ok(ChannelWrite::Stored),
        Ok(AppendOutcome::Held) | Err(rekindle_protocol::ProtocolError::PoolClosed) => {
            Ok(ChannelWrite::Held)
        }
        Err(e) => Err(FilesError::Transport(format!("SMPL channel write: {e}"))),
    }
}

/// The unlocked identity's outbox `channel_post` stream.
fn channel_posts(state: &Arc<AppState>) -> Result<ChannelPosts, FilesError> {
    (state.host.current())
        .and_then(|host| host.channel_posts())
        .ok_or_else(|| FilesError::Db("no unlocked outbox".into()))
}

pub(super) async fn queue_post_impl(
    state: &Arc<AppState>,
    post: &ChannelPost,
) -> Result<(), FilesError> {
    (channel_posts(state)?.queue(post).await).map_err(|e| FilesError::Db(e.to_string()))
}

pub(super) async fn settle_posts_impl(state: &Arc<AppState>, record_key: &str, subkey: u32) {
    let settled = match channel_posts(state) {
        Ok(posts) => posts.settle(record_key, subkey, true).await.map(drop),
        Err(e) => Err(rekindle_outbox::OutboxError::Storage(e.to_string())),
    };
    if let Err(error) = settled {
        tracing::warn!(%error, "settling the attachment posts failed");
    }
}

pub(super) async fn fail_post_impl(state: &Arc<AppState>, message_id: &str) {
    let failed = match channel_posts(state) {
        Ok(posts) => posts.fail(message_id).await.map(drop),
        Err(e) => Err(rekindle_outbox::OutboxError::Storage(e.to_string())),
    };
    if let Err(error) = failed {
        tracing::warn!(%error, message_id, "failing the attachment post failed");
    }
}

pub(super) async fn write_attachment_cached_impl(
    state: &Arc<AppState>,
    community_id: &str,
    channel_log_key: &str,
    slot_index: u32,
    slot_keypair: &str,
    cached: &ChannelAttachmentCached,
) -> Result<(), FilesError> {
    let ctx = build_dht_writer_context(state, community_id)?;
    let written = write_member_attachment_cached(
        ctx.pool.as_ref(),
        channel_log_key,
        slot_index,
        slot_keypair,
        ctx.author_pseudo,
        &ctx.signing_key,
        cached,
    )
    .await;
    held_is_written(written, "AttachmentCached SMPL write")
}

/// A channel write the record pool holds (a miss, or one logout cut short)
/// lands when it can (plan C7.13); an `AttachmentCached` entry carries no
/// delivery status, so held is written.
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
        rekindle_db::repo::messages::channel::insert(
            conn,
            &rekindle_db::repo::messages::channel::ChannelMessageInsert {
                reply_to: None,
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
                // Queued: its outbox `channel_post` row's settle raises it
                // (plan E2.5).
                send_state: rekindle_types::message::SendState::Queued,
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
                Ok((
                    rekindle_db::row::get::<Option<String>>(row, "message_id")?,
                    rekindle_db::row::get_json::<rekindle_files::AttachmentRecordJson>(
                        row,
                        "attachment_json",
                    )?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (message_id, mut record) in rows {
            // The UPDATE addresses a row by its network message id; a row
            // without one is not the posted attachment this path is for.
            let Some(message_id) = message_id else {
                continue;
            };
            if record.attachment_id != attachment_id_hex {
                continue;
            }
            record.local_path = Some(new_path.clone());
            let updated = serde_json::to_string(&record)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
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
