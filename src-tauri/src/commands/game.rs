use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::protocol::ServersRequest;
use rekindle_types::dto::presence::{
    FavouriteServerView, GameTotalView, HiddenGameView, InstalledGameView, OwnGame, OwnPresenceView,
};
use rekindle_types::dto::servers::ServersView;
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

/// Launch a game, to `server` when given (`IpcRequest::GameLaunch`, lead
/// decision (50)). `server` arrives parsed: a text that is not `host:port`
/// fails to deserialize. `source` decides the review: a peer's address is
/// reviewed first, naming it.
#[tauri::command]
pub async fn launch_game(
    game_id: u32,
    server: Option<rekindle_game_detect::launcher::ServerAddress>,
    source: rekindle_types::game_server::JoinSource,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::GameLaunch {
        game_id,
        server,
        source,
    };
    host_call::call(&state, request).await
}

/// The games we can launch (`IpcRequest::InstalledGamesGet`, plan E7.15
/// IV3).
#[tauri::command]
pub async fn get_installed_games(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<InstalledGameView>>, CommandError> {
    host_call::call(&state, IpcRequest::InstalledGamesGet).await
}

/// Invite an accepted friend to join us on a game server
/// (`IpcRequest::GameInviteSend`, plan E7.2). `server` arrives parsed: a
/// text that is not `host:port` fails to deserialize. The outbox id.
#[tauri::command]
pub async fn send_game_invite(
    public_key: String,
    game_id: u32,
    server: rekindle_game_detect::launcher::ServerAddress,
    state: State<'_, SharedState>,
) -> Result<Done<String>, CommandError> {
    let request = IpcRequest::GameInviteSend {
        public_key,
        game_id,
        server,
    };
    host_call::call(&state, request).await
}

/// Our favourite game servers (`IpcRequest::Servers(ServersRequest::FavouritesGet)`, plan
/// E7.2), each with whether an invite may name it.
#[tauri::command]
pub async fn get_favourite_servers(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<FavouriteServerView>>, CommandError> {
    host_call::call(&state, IpcRequest::Servers(ServersRequest::FavouritesGet)).await
}

/// Add or relabel a favourite game server (`ServersRequest::FavouriteAdd`,
/// plan E7.2): `server` arrives parsed. The list comes back.
#[tauri::command]
pub async fn add_favourite_server(
    game_id: u32,
    server: rekindle_game_detect::launcher::ServerAddress,
    label: String,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<FavouriteServerView>>, CommandError> {
    let request = IpcRequest::Servers(ServersRequest::FavouriteAdd {
        game_id,
        server,
        label,
    });
    host_call::call(&state, request).await
}

/// Remove a favourite game server (`ServersRequest::FavouriteRemove`,
/// plan E7.2). The list comes back.
#[tauri::command]
pub async fn remove_favourite_server(
    game_id: u32,
    server: rekindle_game_detect::launcher::ServerAddress,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<FavouriteServerView>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Servers(ServersRequest::FavouriteRemove { game_id, server }),
    )
    .await
}

/// The Servers view (`IpcRequest::ServersView`, plan E7.2 G3): favourites,
/// recent, friends and each community's servers, every row with the Host's
/// verdicts. The Host pushes it again as it changes
/// (`SystemEvent::ServersViewChanged`, `servers-event`).
#[tauri::command]
pub async fn servers_view(
    state: State<'_, SharedState>,
) -> Result<Done<ServersView>, CommandError> {
    host_call::call(&state, IpcRequest::Servers(ServersRequest::View)).await
}

/// Forget a server we visited (`IpcRequest::RecentServerRemove`, plan E7.2
/// G3): `server` arrives parsed. The view comes back.
#[tauri::command]
pub async fn recent_server_remove(
    game_id: u32,
    server: rekindle_game_detect::launcher::ServerAddress,
    state: State<'_, SharedState>,
) -> Result<Done<ServersView>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Servers(ServersRequest::RecentRemove { game_id, server }),
    )
    .await
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
