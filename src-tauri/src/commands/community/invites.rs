use tauri::State;

use crate::db::DbPool;
use crate::services::community_invite_runtime::{
    create_community_invite_inner, list_community_invites_inner, revoke_community_invite_inner,
    InviteCreatedDto, InviteInfoDto,
};
use crate::state::SharedState;

#[tauri::command]
pub async fn create_community_invite(
    community_id: String,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<InviteCreatedDto, String> {
    create_community_invite_inner(
        state.inner(),
        pool.inner(),
        community_id,
        max_uses,
        expires_in_seconds,
    )
    .await
}

#[tauri::command]
pub async fn revoke_community_invite(
    community_id: String,
    code_hash: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    revoke_community_invite_inner(state.inner(), community_id, code_hash).await
}

#[tauri::command]
pub async fn list_community_invites(
    community_id: String,
    pool: State<'_, DbPool>,
) -> Result<Vec<InviteInfoDto>, String> {
    list_community_invites_inner(pool.inner(), community_id).await
}
