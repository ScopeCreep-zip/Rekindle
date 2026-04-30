use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::require_permission;
use super::legacy::rotate_mek_local;

#[tauri::command]
pub async fn rotate_mek(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore: State<'_, crate::keystore::KeystoreHandle>,
) -> Result<(), String> {
    let _ = pool;
    require_permission(state.inner(), &community_id, Permissions::ADMINISTRATOR)?;

    rotate_mek_local(state.inner(), &community_id, &keystore).await?;

    tracing::info!(community = %community_id, "MEK rotated locally");
    Ok(())
}
