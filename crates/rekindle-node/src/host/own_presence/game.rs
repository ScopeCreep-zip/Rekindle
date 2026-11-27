//! The private game presence value we publish to friends.

use rekindle_presence::StatusPublisherDeps;

use super::{subkeys, Game, OwnPresence};

impl OwnPresence {
    /// Write `shown` (the game as the status shows it) to GAME_INFO, sealed.
    pub(super) async fn write_game(&self, shown: Option<&Game>) -> Result<(), String> {
        let bytes = if let Some(game) = shown {
            rekindle_codec::capnp_codec::presence::encode_game_status(
                &rekindle_codec::capnp_codec::presence::GameInfo {
                    game_id: game.id,
                    game_name: game.name.clone(),
                    server_info: game.server_info.clone(),
                    elapsed_seconds: self.elapsed_seconds(game),
                    server_address: game.server_address.clone().map(String::from),
                },
            )
            .map_err(|error| format!("encode game info: {error}"))?
        } else {
            Vec::new()
        };
        let (owner, key) = self
            .presence_key()
            .ok_or("no friend-presence key yet: the game is not written in the clear")?;
        let sealed =
            rekindle_secrets::friend_presence::seal(&key, &owner, subkeys::GAME_INFO, &bytes)
                .map_err(|e| format!("seal game info: {e}"))?;
        self.write_profile(subkeys::GAME_INFO, sealed).await
    }
}
