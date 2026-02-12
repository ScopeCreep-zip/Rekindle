use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::SharedState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    pub game_id: u32,
    pub game_name: String,
    pub elapsed_seconds: u32,
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
            }));
        }
    }
    Ok(None)
}
