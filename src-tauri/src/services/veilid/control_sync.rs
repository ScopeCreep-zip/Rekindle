use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use tauri::Manager;

pub(crate) fn check_gossip_moderation_permission(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: &rekindle_protocol::dht::community::envelope::ControlPayload,
) -> bool {
    use rekindle_protocol::dht::community::envelope::ControlPayload;
    use rekindle_types::permissions;

    let required: u64 = match payload {
        ControlPayload::Kick { .. } => permissions::KICK_MEMBERS,
        ControlPayload::Ban { .. } | ControlPayload::Unban { .. } => permissions::BAN_MEMBERS,
        ControlPayload::TimeoutMember { .. } | ControlPayload::RemoveTimeout { .. } => {
            permissions::TIMEOUT_MEMBERS
        }
        _ => return true,
    };

    let Some(gov) = crate::state_helpers::governance_state(state, community_id) else {
        return false;
    };
    let Ok(bytes) = hex::decode(sender_pseudonym) else {
        return false;
    };
    let Ok(pseudo_bytes): Result<[u8; 32], _> = bytes.try_into() else {
        return false;
    };
    let perms = rekindle_governance::permissions::compute_permissions(
        &rekindle_types::id::PseudonymKey(pseudo_bytes),
        None,
        &gov,
        rekindle_utils::timestamp_secs(),
    );
    if rekindle_governance::permissions::has_capability(perms, required) {
        true
    } else {
        tracing::warn!(
            community = %community_id,
            sender = %sender_pseudonym,
            required = format!("{required:#x}"),
            "gossip moderation: sender lacks required permission — ignoring"
        );
        false
    }
}

pub(crate) fn handle_sync_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    since_timestamp: u64,
) {
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
    let community_id_owned = community_id.to_string();
    let channel_id_owned = channel_id.to_string();
    let channel_id_for_envelope = channel_id.to_string();
    let since_ts = since_timestamp.cast_signed();
    let state = Arc::clone(state);
    let pool = pool.inner().clone();

    tokio::spawn(async move {
        let messages: Vec<rekindle_types::message::SyncedMessage> =
            crate::db_helpers::db_call(&pool, move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT sender_key, body, timestamp, mek_generation, lamport_ts \
                         FROM messages \
                         WHERE owner_key = ? AND conversation_id = ? \
                           AND conversation_type = 'channel' AND timestamp >= ? \
                         ORDER BY timestamp ASC LIMIT 500",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![owner_key, channel_id_owned, since_ts],
                    |row| {
                        Ok(rekindle_types::message::SyncedMessage {
                            sender_key: row.get::<_, String>(0)?,
                            body: row.get::<_, String>(1)?,
                            timestamp: row.get::<_, i64>(2)?,
                            mek_generation: row.get::<_, Option<i64>>(3)?,
                            lamport_ts: row.get::<_, Option<i64>>(4)?,
                        })
                    },
                )?;
                Ok(rows.filter_map(std::result::Result::ok).collect::<Vec<_>>())
            })
            .await
            .unwrap_or_default();

        if messages.is_empty() {
            return;
        }

        tracing::debug!(
            community = %community_id_owned,
            count = messages.len(),
            "responding to sync request"
        );

        let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::SyncResponse {
                channel_id: channel_id_for_envelope,
                messages,
            },
        );
        let _ = crate::services::community::send_to_mesh(&state, &community_id_owned, &envelope);
    });
}

pub(crate) fn handle_sync_response(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    messages: &[rekindle_types::message::SyncedMessage],
) {
    if messages.is_empty() {
        return;
    }

    tracing::info!(
        community = %community_id,
        channel = %channel_id,
        count = messages.len(),
        "merging sync response messages"
    );

    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();

    for message in messages {
        let sender = message.sender_key.clone();
        let body = message.body.clone();
        let timestamp = message.timestamp;
        let mek_generation = message.mek_generation;
        let owner_key = owner_key.clone();
        let channel_id = channel_id.to_string();
        db_fire(pool.inner(), "store sync message", move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO messages \
                 (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
                 VALUES (?, ?, 'channel', ?, ?, ?, 0, ?)",
                rusqlite::params![owner_key, channel_id, sender, body, timestamp, mek_generation],
            )?;
            Ok(())
        });
    }

    crate::event_dispatch::emit_live(
        app_handle,
        "community-event",
        &crate::channels::CommunityEvent::SyncComplete {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_count: messages.len(),
        },
    );
}
