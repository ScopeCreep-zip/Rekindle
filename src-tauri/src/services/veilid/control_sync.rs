use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use tauri::{Emitter, Manager};

pub(crate) fn check_gossip_moderation_permission(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: &rekindle_protocol::dht::community::envelope::ControlPayload,
) -> bool {
    use rekindle_protocol::dht::community::envelope::ControlPayload;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    let required = match payload {
        ControlPayload::Kick { .. } => Permissions::KICK_MEMBERS,
        ControlPayload::Ban { .. } | ControlPayload::Unban { .. } => Permissions::BAN_MEMBERS,
        ControlPayload::TimeoutMember { .. } | ControlPayload::RemoveTimeout { .. } => {
            Permissions::MODERATE_MEMBERS
        }
        _ => return true,
    };

    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return false;
    };

    let sender_role_ids = community.member_roles.get(sender_pseudonym).cloned();
    match sender_role_ids {
        Some(ref role_ids) => {
            let roles_v2: Vec<rekindle_protocol::dht::community::types::RoleEntryV2> = community
                .roles
                .iter()
                .map(
                    |role| rekindle_protocol::dht::community::types::RoleEntryV2 {
                        id: role.id,
                        name: role.name.clone(),
                        color: role.color,
                        permissions: role.permissions,
                        position: role.position,
                        hoist: role.hoist,
                        mentionable: role.mentionable,
                        self_assignable: role.self_assignable,
                    },
                )
                .collect();
            drop(communities);
            let is_owner = crate::state_helpers::governance_state(state, community_id)
                .and_then(|gov| {
                    let pseudo_bytes: [u8; 32] =
                        hex::decode(sender_pseudonym).ok()?.try_into().ok()?;
                    Some(
                        gov.creator.as_ref()
                            == Some(&rekindle_types::id::PseudonymKey(pseudo_bytes)),
                    )
                })
                .unwrap_or(false);
            let permissions =
                rekindle_protocol::dht::community::permissions_v2::calculate_permissions_v2(
                    role_ids,
                    &roles_v2,
                    &[],
                    sender_pseudonym,
                    is_owner,
                    None,
                );
            if permissions.has(required) {
                true
            } else {
                tracing::warn!(
                    community = %community_id,
                    sender = %sender_pseudonym,
                    required = ?required,
                    "gossip moderation: sender lacks required permission — ignoring"
                );
                false
            }
        }
        None => community.known_members.contains(sender_pseudonym),
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
        let messages: Vec<serde_json::Value> = crate::db_helpers::db_call(&pool, move |conn| {
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
                    Ok(serde_json::json!({
                        "sender_key": row.get::<_, String>(0)?,
                        "body": row.get::<_, String>(1)?,
                        "timestamp": row.get::<_, i64>(2)?,
                        "mek_generation": row.get::<_, Option<i64>>(3)?,
                        "lamport_ts": row.get::<_, Option<i64>>(4)?,
                    }))
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
    messages: &[serde_json::Value],
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
        let sender = message["sender_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let body = message["body"].as_str().unwrap_or_default().to_string();
        let timestamp = message["timestamp"].as_i64().unwrap_or_default();
        let mek_generation = message["mek_generation"].as_i64();
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

    let _ = app_handle.emit(
        "community-event",
        crate::channels::CommunityEvent::SyncComplete {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_count: messages.len(),
        },
    );
}
