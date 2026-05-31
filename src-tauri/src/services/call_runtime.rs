//! Phase 23.C — calls-handler Tauri-runtime orchestration lifted from
//! `commands/calls.rs`. Hosts the three mid-call signaling
//! orchestrators (`send_call_media_state_inner`,
//! `send_call_reaction_inner`, `get_missed_calls_inner`) so the Tauri
//! commands stay thin delegations.

use rusqlite::params;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissedCallRow {
    pub call_id: String,
    pub peer_key: String,
    pub kind: u8,
    pub expired_at: i64,
}

pub async fn send_call_media_state_inner(
    state: &SharedState,
    pool: &DbPool,
    call_id: String,
    audio: bool,
    video: bool,
    screen: bool,
) -> Result<(), String> {
    let peer_pubkey = state
        .active_calls
        .get(&call_id)
        .map(|c| c.peer_pubkey.clone())
        .ok_or_else(|| format!("no active call with id {call_id}"))?;
    let payload = MessagePayload::CallMediaState {
        call_id,
        audio,
        video,
        screen,
        timestamp_ms: rekindle_utils::timestamp_ms(),
    };
    crate::services::message_service::send_to_peer_raw(state, pool, &peer_pubkey, &payload)
        .await
        .map_err(|e| format!("send_call_media_state: {e}"))
}

pub async fn send_call_reaction_inner(
    state: &SharedState,
    pool: &DbPool,
    call_id: String,
    emoji: String,
) -> Result<(), String> {
    if emoji.is_empty() || emoji.len() > 32 {
        return Err("emoji must be 1-32 bytes".into());
    }
    let peer_pubkey = state
        .active_calls
        .get(&call_id)
        .map(|c| c.peer_pubkey.clone())
        .ok_or_else(|| format!("no active call with id {call_id}"))?;
    let payload = MessagePayload::CallReaction {
        call_id,
        emoji,
        timestamp_ms: rekindle_utils::timestamp_ms(),
    };
    crate::services::message_service::send_to_peer_raw(state, pool, &peer_pubkey, &payload)
        .await
        .map_err(|e| format!("send_call_reaction: {e}"))
}

pub async fn get_missed_calls_inner(
    state: &SharedState,
    pool: &DbPool,
) -> Result<Vec<MissedCallRow>, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT call_id, peer_key, kind, expired_at FROM missed_calls \
             WHERE owner_key = ?1 ORDER BY expired_at DESC LIMIT 200",
        )?;
        let rows = stmt
            .query_map(params![owner_key], |row| {
                Ok(MissedCallRow {
                    call_id: row.get::<_, String>(0)?,
                    peer_key: row.get::<_, String>(1)?,
                    kind: u8::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                    expired_at: row.get::<_, i64>(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

pub fn mute_caller_temp_inner(state: &SharedState, peer_public_key: String, duration_ms: u64) {
    let expires_at = rekindle_utils::timestamp_ms().saturating_add(duration_ms);
    state
        .temp_call_muted
        .lock()
        .insert(peer_public_key, expires_at);
}
