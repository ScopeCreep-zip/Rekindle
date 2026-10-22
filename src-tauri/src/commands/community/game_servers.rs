//! A community's game servers (plan E3.1 H1): Host calls.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::game_server::GameServerInfo;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn add_game_server(
    state: State<'_, SharedState>,
    community_id: String,
    game_id: String,
    label: String,
    address: String,
) -> Result<Done<String>, CommandError> {
    let request = IpcRequest::GameServerAdd {
        community: community_id,
        game_id,
        label,
        address,
    };
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn remove_game_server(
    state: State<'_, SharedState>,
    community_id: String,
    server_id: String,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::GameServerRemove {
        community: community_id,
        server_id,
    };
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn get_game_servers(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<Vec<GameServerInfo>>, CommandError> {
    let request = IpcRequest::GameServerList {
        community: community_id,
    };
    host_call::call(&state, request).await
}
