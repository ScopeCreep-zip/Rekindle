//! Game invites (plan E7.2): "join me on this server", ratchet-sealed
//! `GameInvite` (0x0170) to one accepted friend. The server is a parsed
//! `ServerAddress` at both ends: the sender's request carries one and the
//! decoder refuses any other text, so nothing unparsed reaches a launch. A
//! received invite is shown, never acted on: joining is the receiver's own
//! action, the Host's `GameLaunch` (`JoinGameServer`, lead decision (50)).
//! An invite says what we play, so it follows the current-game privacy
//! settings: never for a hidden game, and none while `share_current_game`
//! is off.

use rekindle_codec::capnp_codec::direct::{DirectBody, GameInvite};
use rekindle_types::friend::FriendshipState;
use rekindle_types::game_server::ServerAddress;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::{state, FriendError, FriendHost};
use crate::operations::dm;

/// Invite `peer`, an accepted friend, to join us on `server` in `game_id`.
/// The outbox id of the committed frame.
///
/// # Errors
/// `peer` is not an accepted friend, `game_id` is 0, the game is hidden or
/// current-game sharing is off, or the frame could not be committed to the
/// outbox.
pub async fn send(
    host: &dyn FriendHost,
    peer: &str,
    game_id: u32,
    server: ServerAddress,
) -> Result<String, FriendError> {
    if game_id == 0 {
        return Err(FriendError::Invalid("game 0 names no game".into()));
    }
    if state::friend(host, peer).map(|f| f.friendship_state) != Some(FriendshipState::Accepted) {
        return Err(FriendError::NotFound(format!(
            "{peer} is not an accepted friend"
        )));
    }
    let owner = state::owner(host)?;
    let (shared, hidden) = state::db_call!(host, "read the game privacy settings", move |conn| {
        let prefs = rekindle_db::repo::preferences::load(conn, &owner)?;
        let hidden = rekindle_db::repo::hidden_games::is_hidden(conn, &owner, game_id)?;
        Ok((prefs.share_current_game, hidden))
    })
    .await?;
    if !shared {
        return Err(FriendError::Invalid(
            "current-game sharing is off: no game invites".into(),
        ));
    }
    if hidden {
        return Err(FriendError::Invalid(format!("game {game_id} is hidden")));
    }
    let body = DirectBody::GameInvite(GameInvite { game_id, server });
    dm::handshake::enqueue(host, peer, &body).await
}

/// A friend's `GameInvite`: shown only from an accepted friend; anyone
/// else's is dropped. Whether it was handled.
pub(crate) fn on_game_invite(host: &dyn FriendHost, sender: &str, body: &GameInvite) -> bool {
    if state::friend(host, sender).map(|f| f.friendship_state) != Some(FriendshipState::Accepted) {
        tracing::debug!(from = %sender, "GameInvite from a non-friend dropped");
        return true;
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::GameInvite {
        peer_key: sender.to_string(),
        game_id: body.game_id,
        server: body.server.clone(),
    }));
    true
}
