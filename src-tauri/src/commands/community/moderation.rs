use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;

use crate::services::community_audit_runtime::BannedMemberInfo;
use crate::services::community_moderation_runtime::{
    admin_delete_channel_message_inner, ban_member_inner, delete_channel_overwrite_inner,
    remove_community_member_inner, remove_timeout_inner, set_channel_overwrite_inner,
    set_slowmode_inner, timeout_member_inner, unban_member_inner,
};

/// Remove a member from a community.
///
/// Phase 8 — `idempotency_key` (UUID v7) dedupes click-spam. Two rapid
/// clicks would each send a `Kick` control envelope; the second still
/// fans out across the mesh even though the member is already gone
/// locally — wasted bandwidth and a confusing audit trail.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    pseudonym_key: String,
    idempotency_key: uuid::Uuid,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _g = rekindle_lifecycle::TransportGuard::write(&state.lifecycle)
        .map_err(|e| e.to_string())?;
    let s = state.inner().clone();
    let p = pool.inner().clone();
    state
        .idempotency
        .wrap(idempotency_key, || async move {
            remove_community_member_inner(s, p, community_id, pseudonym_key).await
        })
        .await
}

#[tauri::command]
pub async fn timeout_member(
    community_id: String,
    pseudonym_key: String,
    duration_seconds: u64,
    reason: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    timeout_member_inner(
        state.inner(),
        pool.inner(),
        community_id,
        pseudonym_key,
        duration_seconds,
        reason,
    )
    .await
}

#[tauri::command]
pub async fn remove_timeout(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    remove_timeout_inner(state.inner(), pool.inner(), community_id, pseudonym_key).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments, reason = "Tauri command surface — matches PermissionOverwrite shape")]
pub async fn set_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    allow: u64,
    deny: u64,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    set_channel_overwrite_inner(
        state.inner(),
        pool.inner(),
        community_id,
        channel_id,
        target_type,
        target_id,
        allow,
        deny,
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
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    delete_channel_overwrite_inner(
        state.inner(),
        pool.inner(),
        community_id,
        channel_id,
        target_type,
        target_id,
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
) -> Result<(), String> {
    ban_member_inner(state.inner(), community_id, pseudonym_key).await
}

#[tauri::command]
pub async fn unban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    unban_member_inner(state.inner(), community_id, pseudonym_key).await
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
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    admin_delete_channel_message_inner(
        state.inner(),
        pool.inner(),
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
    pool: State<'_, DbPool>,
) -> Result<u32, String> {
    crate::services::community_moderation_bulk::bulk_delete_channel_messages_inner(
        state.inner(),
        pool.inner(),
        community_id,
        channel_id,
        message_ids,
        reason,
    )
    .await
}

/// Get the list of banned members for a community from the merged governance state.
#[tauri::command]
pub async fn get_ban_list(
    community_id: String,
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
) -> Result<Vec<BannedMemberInfo>, String> {
    crate::services::community_audit_runtime::get_ban_list_inner(state.inner(), &community_id)
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
