use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;

use crate::state::SharedState;

use rekindle_types::dto::community::CommunityBanView;

use crate::services::community_moderation_runtime::{
    admin_delete_channel_message_inner, set_slowmode_inner,
};

/// Remove a member from a community without barring return.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Kick {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }),
    )
    .await
}

#[tauri::command]
pub async fn timeout_member(
    community_id: String,
    pseudonym_key: String,
    duration_seconds: u64,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Timeout {
            community: community_id,
            target_pseudonym: pseudonym_key,
            duration_seconds,
            reason,
        }),
    )
    .await
}

#[tauri::command]
pub async fn remove_timeout(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::TimeoutRemove {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }),
    )
    .await
}

#[tauri::command]
pub async fn set_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    allow: u64,
    deny: u64,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::ChannelOverwriteSet {
            community: community_id,
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
        }),
    )
    .await
}

#[tauri::command]
pub async fn delete_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::ChannelOverwriteDelete {
            community: community_id,
            channel_id,
            target_type,
            target_id,
        }),
    )
    .await
}

#[tauri::command]
pub async fn set_slowmode(
    community_id: String,
    channel_id: String,
    seconds: u32,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    set_slowmode_inner(state.inner(), community_id, channel_id, seconds).await
}

#[tauri::command]
pub async fn ban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Ban {
            community: community_id,
            target_pseudonym: pseudonym_key,
            reason: None,
        }),
    )
    .await
}

#[tauri::command]
pub async fn unban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Unban {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }),
    )
    .await
}

/// Admin-deletes a single channel message: writes `GovernanceEntry::AdminDelete`
/// (durable tombstone), gossips `ControlPayload::MessageDeleted` (UI update on
/// online peers), purges the local SQLite row, and emits a local
/// `CommunityEvent::MessageDeleted` so the actor's UI updates immediately.
#[tauri::command]
pub async fn admin_delete_channel_message(
    community_id: String,
    channel_id: String,
    message_id: String,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let pool = state.db.current()?;
    admin_delete_channel_message_inner(
        state.inner(),
        &pool,
        community_id,
        channel_id,
        message_id,
        reason,
    )
    .await
}

/// Admin bulk-deletes up to `BULK_DELETE_CAP` channel messages. Per architecture
/// §16.7, callers must send ≤100 ids per request — caller code (frontend toolbar)
/// can chunk larger selections itself. Each id produces an `AdminDelete`
/// governance entry, a gossip `MessageDeleted`, a local SQLite delete, and a
/// local UI event. Returns the count of successful deletions; per-id failures
/// are logged but do not abort the batch (best-effort: peers see what we
/// successfully wrote).
#[tauri::command]
pub async fn bulk_delete_channel_messages(
    community_id: String,
    channel_id: String,
    message_ids: Vec<String>,
    reason: Option<String>,
    state: State<'_, SharedState>,
) -> Result<u32, String> {
    let pool = state.db.current()?;
    crate::services::community_moderation_bulk::bulk_delete_channel_messages_inner(
        state.inner(),
        &pool,
        community_id,
        channel_id,
        message_ids,
        reason,
    )
    .await
}

/// The community's bans, from the merged governance state.
#[tauri::command]
pub async fn get_ban_list(
    community_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<CommunityBanView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::BanList {
            community: community_id,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use crate::services::community_moderation_runtime::{message_id_to_bytes, BULK_DELETE_CAP};

    #[test]
    fn message_id_to_bytes_round_trips_uuid() {
        let id = uuid::Uuid::new_v4();
        let formatted = format!("msg_{}", id.as_simple());
        let bytes = message_id_to_bytes(&formatted);
        assert_eq!(bytes, *id.as_bytes());
    }

    #[test]
    fn message_id_to_bytes_handles_bare_uuid() {
        let id = uuid::Uuid::new_v4();
        let bytes = message_id_to_bytes(id.as_simple().to_string().as_str());
        assert_eq!(bytes, *id.as_bytes());
    }

    #[test]
    fn message_id_to_bytes_falls_back_to_zero_on_garbage() {
        assert_eq!(message_id_to_bytes("not-a-uuid"), [0u8; 16]);
        assert_eq!(message_id_to_bytes(""), [0u8; 16]);
    }

    #[test]
    fn bulk_delete_cap_is_one_hundred() {
        assert_eq!(BULK_DELETE_CAP, 100);
    }
}
