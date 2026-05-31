use tauri::State;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::db::DbPool;
use crate::services::community_game_servers_runtime::{
    add_game_server_inner, get_game_servers_inner,
};
use crate::state::SharedState;

use super::helpers::require_permission;

pub use crate::channels::community_channel::GameServerInfoDto;

#[tauri::command]
pub async fn add_game_server(
    state: State<'_, SharedState>,
    community_id: String,
    game_id: String,
    label: String,
    address: String,
) -> Result<String, String> {
    add_game_server_inner(state.inner(), &community_id, game_id, label, address)
}

#[tauri::command]
pub async fn remove_game_server(
    state: State<'_, SharedState>,
    community_id: String,
    server_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::GameServerRemoved { server_id }),
    )
}

#[tauri::command]
pub async fn get_game_servers(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<GameServerInfoDto>, String> {
    get_game_servers_inner(state.inner(), pool.inner(), community_id).await
}
