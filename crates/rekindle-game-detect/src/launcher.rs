//! Game launch support via URI schemes.
//!
//! Uses `steam://connect/{addr}` for Steam games and custom `connect_template`
//! URIs from the game database for non-Steam games. The `open` crate invokes
//! the OS default handler, so Steam handles `steam://` URIs cross-platform.

use crate::database::GameDatabase;
use crate::error::GameDetectError;

/// Launch a game and connect to the given server address.
///
/// Resolution order:
/// 1. `connect_template` from the database entry (if present)
/// 2. `steam://connect/{addr}` if the game has a `steam_app_id`
/// 3. Error if neither is available
pub fn launch_to_server(
    db: &GameDatabase,
    game_id: u32,
    server_address: &str,
) -> Result<(), GameDetectError> {
    let entry = db
        .lookup_by_id(game_id)
        .ok_or_else(|| GameDetectError::DatabaseError(format!("unknown game_id {game_id}")))?;

    let url = if let Some(ref template) = entry.connect_template {
        template.replace("{addr}", server_address)
    } else if entry.steam_app_id.is_some() {
        format!("steam://connect/{server_address}")
    } else {
        return Err(GameDetectError::DatabaseError(
            "no launch method for this game".into(),
        ));
    };

    open::that(&url).map_err(GameDetectError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_unknown_game_errors() {
        let db = GameDatabase::empty();
        let result = launch_to_server(&db, 9999, "1.2.3.4:27015");
        assert!(result.is_err());
    }
}
