//! A community's game servers (plan E3.1 H1): Host calls.

use rekindle_ipc::protocol::ActivityRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::game_server::GameServerInfo;
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    pub async fn add_game_server(
        state: State<'_, SharedState>,
        community_id: String,
        game_id: String,
        label: String,
        address: String,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::GameServerAdd {
                community: community_id,
                game_id,
                label,
                address,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn remove_game_server(
        state: State<'_, SharedState>,
        community_id: String,
        server_id: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::GameServerRemove {
                community: community_id,
                server_id,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn get_game_servers(
        state: State<'_, SharedState>,
        community_id: String,
    ) -> Result<Done<Vec<GameServerInfo>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::GameServerList {
                community: community_id,
            },
        ))
    }
}
