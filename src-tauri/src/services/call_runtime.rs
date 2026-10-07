//! Phase 23.C — calls-handler Tauri-runtime orchestration lifted from
//! `commands/calls.rs`. Hosts the three mid-call signaling
//! orchestrators (`send_call_media_state_inner`,
//! `send_call_reaction_inner`, `get_missed_calls_inner`) so the Tauri
//! commands stay thin delegations.

use rusqlite::params;

use rekindle_codec::message::envelope::MessagePayload;

use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissedCallRow {
    pub call_id: String,
    pub peer_key: String,
    pub kind: u8,
    pub expired_at: i64,
}

/// A live 1:1 call as a call window shows it on open.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveCallDto {
    pub call_id: String,
    pub peer_key: String,
    pub display_name: String,
    /// `"audio"` or `"video"`.
    pub kind: &'static str,
    pub expires_at_ms: u64,
    /// The call was accepted (connecting or active), as opposed to still
    /// ringing out.
    pub connected: bool,
}

/// Snapshot of one live 1:1 call, or `None` once it has ended.
pub fn get_active_call_inner(
    state: &SharedState,
    call_id: &str,
) -> Result<Option<ActiveCallDto>, String> {
    let call_id = rekindle_types::key_format::call_id(call_id)
        .map_err(|e| format!("invalid call id: {e}"))?;
    let Some(call) = state.active_calls.get(call_id.as_str()) else {
        return Ok(None);
    };
    let connected = match call.status {
        rekindle_calls::CallStatus::Connecting | rekindle_calls::CallStatus::Active => true,
        rekindle_calls::CallStatus::Outgoing
        | rekindle_calls::CallStatus::Incoming
        | rekindle_calls::CallStatus::Missed => false,
    };
    let display_name = crate::state_helpers::friend_display_name(state, &call.peer_pubkey)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{}…", rekindle_utils::text::prefix(&call.peer_pubkey, 12)));
    Ok(Some(ActiveCallDto {
        call_id: call.call_id.clone(),
        peer_key: call.peer_pubkey.clone(),
        display_name,
        kind: match call.kind {
            rekindle_calls::CallKind::Audio => "audio",
            rekindle_calls::CallKind::Video => "video",
        },
        expires_at_ms: call.expires_at_ms,
        connected,
    }))
}

pub async fn send_call_media_state_inner(
    state: &SharedState,
    pool: &Db,
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
    crate::services::message_service::send_to_peer(state, pool, &peer_pubkey, &payload)
        .await
        .map_err(|e| format!("send_call_media_state: {e}"))
}

pub async fn send_call_reaction_inner(
    state: &SharedState,
    pool: &Db,
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
    crate::services::message_service::send_to_peer(state, pool, &peer_pubkey, &payload)
        .await
        .map_err(|e| format!("send_call_reaction: {e}"))
}

pub async fn get_missed_calls_inner(
    state: &SharedState,
    pool: &Db,
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
