//! Phase 23.D.4 — SQLite persist + retry-queue bodies extracted from
//! `deps_impl.rs`. Each helper resolves the current owner_key and
//! delegates to `crate::message_repo` or executes the
//! channel_slowmode_state / channels SQL directly via the Db.

use rekindle_channel::deps::{ChannelMessageRow, ChannelSendOutcome, ThreadInfoSnapshot};
use rekindle_channel::error::ChannelError;

use crate::channels::community_channel::ThreadInfoDto;
use crate::db_helpers::{db_call, db_fire};
use crate::state_helpers;

use super::ChannelAdapter;

pub(super) async fn persist_sent_message_impl(
    adapter: &ChannelAdapter,
    channel_id: &str,
    outcome: &ChannelSendOutcome,
    body: &str,
) -> Result<(), ChannelError> {
    let owner_key =
        state_helpers::current_owner_key(&adapter.state).map_err(ChannelError::Adapter)?;
    let channel_id = channel_id.to_string();
    let sender_key = outcome.sender_pseudonym_hex.clone();
    let body = body.to_string();
    let message_id = outcome.message_id.clone();
    let mek_generation = outcome.mek_generation;
    let timestamp_ms = outcome.timestamp_ms;
    let lamport_ts = outcome.lamport_ts;
    db_call(&adapter.pool, move |conn| {
        crate::message_repo::insert_channel_message_with_protocol_metadata(
            conn,
            &owner_key,
            &channel_id,
            &sender_key,
            &body,
            timestamp_ms,
            true,
            Some(i64::try_from(mek_generation).unwrap_or(i64::MAX)),
            &message_id,
            lamport_ts,
            false,
        )
    })
    .await
    .map_err(ChannelError::Adapter)
}

pub(super) async fn persist_forwarded_message_impl(
    adapter: &ChannelAdapter,
    channel_id: &str,
    outcome: &ChannelSendOutcome,
    body: &str,
    original_author_pseudonym: &str,
) -> Result<(), ChannelError> {
    let owner_key =
        state_helpers::current_owner_key(&adapter.state).map_err(ChannelError::Adapter)?;
    let channel_id = channel_id.to_string();
    let sender_key = outcome.sender_pseudonym_hex.clone();
    let body = body.to_string();
    let message_id = outcome.message_id.clone();
    let mek_generation = outcome.mek_generation;
    let timestamp_ms = outcome.timestamp_ms;
    let lamport_ts = outcome.lamport_ts;
    let original_author = original_author_pseudonym.to_string();
    db_call(&adapter.pool, move |conn| {
        crate::message_repo::insert_channel_message_with_full_metadata(
            conn,
            &owner_key,
            &channel_id,
            &sender_key,
            &body,
            timestamp_ms,
            true,
            Some(i64::try_from(mek_generation).unwrap_or(i64::MAX)),
            &message_id,
            lamport_ts,
            false,
            Some(&original_author),
        )
    })
    .await
    .map_err(ChannelError::Adapter)
}

pub(super) fn persist_channel_sequence_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
    sequence: u64,
) -> Result<(), ChannelError> {
    let owner_key =
        state_helpers::current_owner_key(&adapter.state).map_err(ChannelError::Adapter)?;
    let community_id = community_id.to_string();
    let channel_id = channel_id.to_string();
    let sequence_i64 = i64::try_from(sequence).unwrap_or(i64::MAX);
    db_fire(&adapter.pool, "persist channel sequence", move |conn| {
        conn.execute(
            "UPDATE channels SET my_sequence = ?1 WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
            rusqlite::params![sequence_i64, owner_key, community_id, channel_id],
        )?;
        Ok(())
    });
    Ok(())
}

pub(super) fn persist_slowmode_state_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) -> Result<(), ChannelError> {
    let owner_key =
        state_helpers::current_owner_key(&adapter.state).map_err(ChannelError::Adapter)?;
    let community_id = community_id.to_string();
    let channel_id = channel_id.to_string();
    db_fire(
        &adapter.pool,
        "persist channel_slowmode_state",
        move |conn| {
            conn.execute(
                "INSERT INTO channel_slowmode_state \
             (owner_key, community_id, channel_id, last_send_ms) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
               last_send_ms = excluded.last_send_ms",
                rusqlite::params![owner_key, community_id, channel_id, now_ms],
            )?;
            Ok(())
        },
    );
    Ok(())
}

pub(super) async fn find_channel_message_by_id_impl(
    adapter: &ChannelAdapter,
    channel_id: &str,
    message_id: &str,
) -> Option<ChannelMessageRow> {
    let owner_key = state_helpers::current_owner_key(&adapter.state).ok()?;
    let channel_id = channel_id.to_string();
    let message_id = message_id.to_string();
    let row = db_call(&adapter.pool, move |conn| {
        crate::message_repo::find_channel_message_by_id(conn, &owner_key, &channel_id, &message_id)
    })
    .await
    .ok()
    .flatten()?;
    Some(ChannelMessageRow {
        sender_key: row.sender_key,
        body: row.body,
    })
}

pub(super) async fn persist_thread_row_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    thread: &ThreadInfoSnapshot,
) -> Result<(), ChannelError> {
    let dto = ThreadInfoDto {
        id: thread.id.clone(),
        channel_id: thread.channel_id.clone(),
        name: thread.name.clone(),
        starter_message_id: thread.starter_message_id.clone(),
        creator_pseudonym: thread.creator_pseudonym.clone(),
        forum_tag: thread.forum_tag.clone(),
        created_at: thread.created_at,
        archived: thread.archived,
        auto_archive_seconds: thread.auto_archive_seconds,
        last_message_at: thread.last_message_at,
        message_count: thread.message_count,
    };
    crate::services::community::threads_store::persist_thread_row(
        &adapter.state,
        &adapter.pool,
        community_id,
        &dto,
    )
    .await
    .map_err(ChannelError::Adapter)
}

pub(super) async fn load_thread_metadata_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    thread_id: &str,
) -> Option<ThreadInfoSnapshot> {
    let owner_key = state_helpers::current_owner_key(&adapter.state).ok()?;
    let dto = crate::services::community::threads_store::load_thread_metadata(
        &adapter.pool,
        &owner_key,
        community_id,
        thread_id,
    )
    .await
    .ok()
    .flatten()?;
    Some(ThreadInfoSnapshot {
        id: dto.id,
        channel_id: dto.channel_id,
        name: dto.name,
        starter_message_id: dto.starter_message_id,
        creator_pseudonym: dto.creator_pseudonym,
        forum_tag: dto.forum_tag,
        created_at: dto.created_at,
        archived: dto.archived,
        auto_archive_seconds: dto.auto_archive_seconds,
        last_message_at: dto.last_message_at,
        message_count: dto.message_count,
    })
}
