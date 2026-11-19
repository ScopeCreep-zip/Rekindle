use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::presence::{GameTotalView, HiddenGameView, OwnGame, OwnPresenceView};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// The game we are playing, from our presence (`IpcRequest::PresenceGet`).
#[tauri::command]
pub async fn get_game_status(
    state: State<'_, SharedState>,
) -> Result<Done<Option<OwnGame>>, CommandError> {
    let presence: Done<OwnPresenceView> = host_call::call(&state, IpcRequest::PresenceGet).await?;
    Ok(Done {
        value: presence.value.game,
        undo: presence.undo,
    })
}

/// Resolve a numeric game ID to a human-readable game name from the bundled database.
#[tauri::command]
pub async fn get_game_name(game_id: u32) -> Option<String> {
    let db = rekindle_game_detect::GameDatabase::bundled();
    db.lookup_by_id(game_id).map(|e| e.name.clone())
}

/// Launch a game and connect to a server. The address came from a peer, so
/// it is parsed first: only a `host:port` reaches the URI the OS opens
/// (plan E1.1).
///
/// Uses `steam://connect/{addr}` for Steam games or a custom connect template
/// from the game database.
#[tauri::command]
pub async fn launch_game_to_server(game_id: u32, server_address: String) -> Result<(), String> {
    let server = rekindle_game_detect::launcher::ServerAddress::parse(&server_address)
        .map_err(|e| format!("not a server address: {e}"))?;
    let db = rekindle_game_detect::GameDatabase::bundled();
    rekindle_game_detect::launcher::launch_to_server(&db, game_id, &server)
        .map_err(|e| e.to_string())
}

/// Our time in each detected game, the most recent first
/// (`IpcRequest::GameTotalsGet`, plan E7.2).
#[tauri::command]
pub async fn get_game_totals(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<GameTotalView>>, CommandError> {
    host_call::call(&state, IpcRequest::GameTotalsGet).await
}

/// A friend's time in each game, as they last shared it
/// (`IpcRequest::FriendGameTotalsGet`, plan E7.2); empty when they share
/// none.
#[tauri::command]
pub async fn get_friend_game_totals(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<GameTotalView>>, CommandError> {
    host_call::call(&state, IpcRequest::FriendGameTotalsGet { public_key }).await
}

/// The games hidden from everyone (`IpcRequest::HiddenGamesGet`, plan E7.2).
#[tauri::command]
pub async fn get_hidden_games(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<HiddenGameView>>, CommandError> {
    host_call::call(&state, IpcRequest::HiddenGamesGet).await
}

/// Hide a game from everyone (`IpcRequest::GameHide`, plan E7.2): the hide
/// list comes back.
#[tauri::command]
pub async fn hide_game(
    game_id: u32,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<HiddenGameView>>, CommandError> {
    host_call::call(&state, IpcRequest::GameHide { game_id }).await
}

/// Take a game off the hide list (`IpcRequest::GameUnhide`, plan E7.2, a
/// trust-changing intent the Host reviews): the hide list comes back.
#[tauri::command]
pub async fn unhide_game(
    game_id: u32,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<HiddenGameView>>, CommandError> {
    host_call::call(&state, IpcRequest::GameUnhide { game_id }).await
}
