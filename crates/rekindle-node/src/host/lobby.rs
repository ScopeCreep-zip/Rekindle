//! The Host's side of a group lobby request (plan E7.2 G6, SK2-G §5): the
//! game and server come from what we show, never from the window.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::game_server::ServerAddress;

use crate::daemon::dispatch::DaemonContext;

/// Our shown game's id, the game a lobby we host is for.
///
/// # Errors
/// 409 while we show no game.
pub(crate) fn own_game(ctx: &DaemonContext) -> Result<u32, IpcResponse> {
    ctx.own_presence
        .view()
        .game
        .map(|g| g.game_id)
        .filter(|id| *id != 0)
        .ok_or_else(|| IpcResponse::error(409, "LobbyHost: we show no game to host a lobby for"))
}

/// The server a launch names: `named`, else our shown game's
/// rich-presence server.
///
/// # Errors
/// 409 when none is named and our game reports none.
pub(crate) fn launch_server(
    ctx: &DaemonContext,
    named: Option<&ServerAddress>,
) -> Result<ServerAddress, IpcResponse> {
    if let Some(server) = named {
        return Ok(server.clone());
    }
    ctx.own_presence
        .view()
        .game
        .and_then(|g| g.server_address)
        .and_then(|s| ServerAddress::parse(&s).ok())
        .ok_or_else(|| {
            IpcResponse::error(
                409,
                "LobbyLaunch: no server named and our game reports none",
            )
        })
}
