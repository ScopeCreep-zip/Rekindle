//! A game launch's review (lead decision (50), ADR 0019): the class follows
//! the launch's `JoinSource`. A peer chose the address (friend presence, an
//! invite, a community's list): `JoinGameServer`, TrustChanging, the review
//! naming the game and `host:port` and that our IP address reaches it. Our
//! own favourite (checked against `favourite_servers`, so a caller cannot
//! claim one to skip the review) or an installed game: Routine.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::game_server::{JoinSource, ServerAddress};
use rekindle_types::intent::{Fact, IntentKind, Subject};

use crate::daemon::dispatch::DaemonContext;

/// Fill `s` for `GameLaunch`.
///
/// # Errors
/// `source` is `Favourite` and the server is not one of ours, or the
/// favourites cannot be read.
pub(super) async fn selection(
    ctx: &DaemonContext,
    game_id: u32,
    server: Option<&ServerAddress>,
    source: JoinSource,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    let game = rekindle_game_detect::GameDatabase::shared()
        .lookup_by_id(game_id)
        .map_or_else(|| format!("Game {game_id}"), |entry| entry.name.clone());
    let Some(server) = server else {
        s.subject = Subject::Named { name: game };
        return Ok(());
    };
    s.subject = Subject::Named {
        name: format!("{game} at {server}"),
    };
    if source == JoinSource::Favourite {
        if !is_favourite(ctx, game_id, server).await? {
            return Err(IpcResponse::error(404, "not one of our favourite servers"));
        }
    } else if source.peer_supplied() {
        s.kind = IntentKind::JoinGameServer;
        s.facts.push(Fact::IpAddressLeaves);
    }
    Ok(())
}

async fn is_favourite(
    ctx: &DaemonContext,
    game_id: u32,
    server: &ServerAddress,
) -> Result<bool, IpcResponse> {
    let db = ctx.require_db()?;
    let owner = ctx.require_owner()?;
    let address = server.as_str().to_owned();
    db.call(move |conn| rekindle_db::repo::favourite_servers::list(conn, &owner))
        .await
        .map(|list| {
            list.iter()
                .any(|f| f.game_id == game_id && f.address == address)
        })
        .map_err(|e| IpcResponse::error(500, format!("favourite servers not read: {e}")))
}

#[cfg(test)]
#[path = "game_launch_tests.rs"]
mod tests;
