use tauri::State;

use crate::commands::chat::Message;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::{random_nonce, require_permission};

pub use crate::channels::community_channel::ThreadInfoDto;

#[tauri::command]
pub async fn create_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
    name: String,
    starter_message_id: String,
) -> Result<String, String> {
    require_permission(
        state.inner(),
        &community_id,
        Permissions::CREATE_PUBLIC_THREADS,
    )?;
    let _ = pool;
    let thread_id = format!("thr_{}", hex::encode(random_nonce(8)));
    let creator_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let now = rekindle_utils::timestamp_secs();
    let thread = ThreadInfoDto {
        id: thread_id.clone(),
        channel_id,
        name,
        starter_message_id,
        creator_pseudonym,
        created_at: now,
        archived: false,
        auto_archive_seconds: 86_400,
        last_message_at: now,
        message_count: 0,
    };

    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::ThreadCreated {
            thread: serde_json::to_value(&thread).map_err(|e| format!("serialize thread: {e}"))?,
        }),
    )?;

    Ok(thread_id)
}

#[tauri::command]
pub async fn get_channel_threads(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    channel_id: String,
) -> Result<Vec<ThreadInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, channel_id, name, starter_message_id, creator_pseudonym, \
                    created_at, archived, auto_archive_seconds, last_message_at, message_count \
             FROM community_threads \
             WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3 \
             ORDER BY last_message_at DESC",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![owner_key, community_id, channel_id],
            |row| {
                Ok(ThreadInfoDto {
                    id: row.get(0)?,
                    channel_id: row.get(1)?,
                    name: row.get(2)?,
                    starter_message_id: row.get(3)?,
                    creator_pseudonym: row.get(4)?,
                    created_at: row.get::<_, i64>(5).unwrap_or(0).cast_unsigned(),
                    archived: row.get::<_, i32>(6).unwrap_or(0) != 0,
                    auto_archive_seconds: row.get::<_, i32>(7).unwrap_or(0).cast_unsigned(),
                    last_message_at: row.get::<_, i64>(8).unwrap_or(0).cast_unsigned(),
                    message_count: row.get::<_, i32>(9).unwrap_or(0).cast_unsigned(),
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

#[tauri::command]
pub async fn send_thread_message(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
    body: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::SEND_MESSAGES)?;
    let _ = pool;
    let (ciphertext, mek_generation) = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(&community_id).ok_or_else(|| {
            "MEK not available — rejoin the community or wait for MEK delivery".to_string()
        })?;
        let ct = mek
            .encrypt(body.as_bytes())
            .map_err(|e| format!("MEK encryption failed: {e}"))?;
        (ct, mek.generation())
    };

    let sender_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let message_id = format!("tmsg_{}", hex::encode(random_nonce(8)));
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::ThreadMessageReceived {
            thread_id,
            message_id,
            sender_pseudonym,
            ciphertext,
            mek_generation,
            timestamp: rekindle_utils::timestamp_secs(),
            reply_to_id: None,
        }),
    )
}

#[tauri::command]
pub async fn get_thread_messages(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
    limit: u32,
    before_timestamp: Option<u64>,
) -> Result<Vec<Message>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let lim = i64::from(limit.min(200));
    db_call(pool.inner(), move |conn| {
        let before_ts = before_timestamp.map_or(i64::MAX, u64::cast_signed);
        let mut stmt = conn.prepare(
            "SELECT message_id, sender_pseudonym, body, timestamp, reply_to_id \
             FROM thread_messages \
             WHERE owner_key = ?1 AND community_id = ?2 AND thread_id = ?3 AND timestamp < ?4 \
             ORDER BY timestamp DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![owner_key, community_id, thread_id, before_ts, lim],
            |row| {
                let sender: String = row.get(1)?;
                let is_own = sender == my_pseudonym;
                Ok(Message {
                    id: 0,
                    sender_id: sender,
                    body: row.get(2)?,
                    decryption_failed: false,
                    automod_blurred: false,
                    timestamp: row.get(3)?,
                    is_own,
                    server_message_id: row.get(0)?,
                    reactions: None,
                    pinned: None,
                    poll: None,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

#[tauri::command]
pub async fn archive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_THREADS)?;
    let _ = pool;
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::ThreadArchived {
            thread_id,
            archived: true,
        }),
    )
}

#[tauri::command]
pub async fn unarchive_thread(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    thread_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_THREADS)?;
    let _ = pool;
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::ThreadArchived {
            thread_id,
            archived: false,
        }),
    )
}
