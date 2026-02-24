use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::SharedState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    pub game_id: u32,
    pub game_name: String,
    pub elapsed_seconds: u32,
    pub server_address: Option<String>,
}

#[tauri::command]
pub async fn get_game_status(state: State<'_, SharedState>) -> Result<Option<GameStatus>, String> {
    let detector = state.game_detector.lock();
    if let Some(ref handle) = *detector {
        if let Some(ref game) = handle.current_game {
            return Ok(Some(GameStatus {
                game_id: game.game_id,
                game_name: game.game_name.clone(),
                elapsed_seconds: game.elapsed_seconds,
                server_address: game.server_address.clone(),
            }));
        }
    }
    Ok(None)
}

/// Resolve a numeric game ID to a human-readable game name from the bundled database.
#[tauri::command]
pub async fn get_game_name(game_id: u32) -> Option<String> {
    let db = rekindle_game_detect::GameDatabase::bundled();
    db.lookup_by_id(game_id).map(|e| e.name.clone())
}

/// Launch a game and connect to a specific server address.
///
/// Uses `steam://connect/{addr}` for Steam games or a custom connect template
/// from the game database.
#[tauri::command]
pub async fn launch_game_to_server(game_id: u32, server_address: String) -> Result<(), String> {
    let db = rekindle_game_detect::GameDatabase::bundled();
    rekindle_game_detect::launcher::launch_to_server(&db, game_id, &server_address)
        .map_err(|e| e.to_string())
}
