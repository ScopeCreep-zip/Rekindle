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

host_command! {
    state;
    /// Our presence as the Host holds it (`IpcRequest::PresenceGet`): the one
    /// source every window hydrates our status from (plan E7.4).
    pub async fn get_own_presence(
        state: State<'_, SharedState>,
    ) -> Result<Done<OwnPresenceView>, CommandError> {
        IpcRequest::PresenceGet
    }
}

/// Resolve a numeric game ID to a human-readable game name from the bundled database.
#[tauri::command]
pub async fn get_game_name(game_id: u32) -> Option<String> {
    let db = rekindle_game_detect::GameDatabase::bundled();
    db.lookup_by_id(game_id).map(|e| e.name.clone())
}

host_command! {
    state;
    /// Launch a game, to `server` when given (`IpcRequest::GameLaunch`, lead
    /// decision (50)). `server` arrives parsed: a text that is not `host:port`
    /// fails to deserialize. `source` decides the review: a peer's address is
    /// reviewed first, naming it.
    pub async fn launch_game(
        game_id: u32,
        server: Option<rekindle_game_detect::launcher::ServerAddress>,
        source: rekindle_types::game_server::JoinSource,
        state: State<'_, SharedState>,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::GameLaunch {
            game_id,
            server,
            source,
        }
    }
}

host_command! {
    state;
    /// The games we can launch (`IpcRequest::InstalledGamesGet`, plan E7.15
    /// IV3).
    pub async fn get_installed_games(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<InstalledGameView>>, CommandError> {
        IpcRequest::InstalledGamesGet
    }
}

host_command! {
    state;
    /// Invite an accepted friend to join us on a game server
    /// (`IpcRequest::GameInviteSend`, plan E7.2). `server` arrives parsed: a
    /// text that is not `host:port` fails to deserialize. The outbox id.
    pub async fn send_game_invite(
        public_key: String,
        game_id: u32,
        server: rekindle_game_detect::launcher::ServerAddress,
        state: State<'_, SharedState>,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::GameInviteSend {
            public_key,
            game_id,
            server,
        }
    }
}

host_command! {
    state;
    /// Our favourite game servers (`IpcRequest::Servers(ServersRequest::FavouritesGet)`, plan
    /// E7.2), each with whether an invite may name it.
    pub async fn get_favourite_servers(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<FavouriteServerView>>, CommandError> {
        IpcRequest::Servers(ServersRequest::FavouritesGet)
    }
}

host_command! {
    state;
    /// Add or relabel a favourite game server (`ServersRequest::FavouriteAdd`,
    /// plan E7.2): `server` arrives parsed. The list comes back.
    pub async fn add_favourite_server(
        game_id: u32,
        server: rekindle_game_detect::launcher::ServerAddress,
        label: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<FavouriteServerView>>, CommandError> {
        IpcRequest::Servers(ServersRequest::FavouriteAdd {
            game_id,
            server,
            label,
        })
    }
}

host_command! {
    state;
    /// Remove a favourite game server (`ServersRequest::FavouriteRemove`,
    /// plan E7.2). The list comes back.
    pub async fn remove_favourite_server(
        game_id: u32,
        server: rekindle_game_detect::launcher::ServerAddress,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<FavouriteServerView>>, CommandError> {
        IpcRequest::Servers(ServersRequest::FavouriteRemove { game_id, server })
    }
}

host_command! {
    state;
    /// The Servers view (`IpcRequest::ServersView`, plan E7.2 G3): favourites,
    /// recent, friends and each community's servers, every row with the Host's
    /// verdicts. The Host pushes it again as it changes
    /// (`SystemEvent::ServersViewChanged`, `servers-event`).
    pub async fn servers_view(
        state: State<'_, SharedState>,
    ) -> Result<Done<ServersView>, CommandError> {
        IpcRequest::Servers(ServersRequest::View)
    }
}

host_command! {
    state;
    /// Forget a server we visited (`IpcRequest::RecentServerRemove`, plan E7.2
    /// G3): `server` arrives parsed. The view comes back.
    pub async fn recent_server_remove(
        game_id: u32,
        server: rekindle_game_detect::launcher::ServerAddress,
        state: State<'_, SharedState>,
    ) -> Result<Done<ServersView>, CommandError> {
        IpcRequest::Servers(ServersRequest::RecentRemove { game_id, server })
    }
}

host_command! {
    state;
    /// Ask every server the Servers view lists what it is running
    /// (`IpcRequest::ServersRefresh`, plan E7.2 G4): the user's Refresh only, as
    /// a query shows our IP address to each server. `started` is false while a
    /// refresh runs; the answers arrive through `servers-event`.
    pub async fn servers_refresh(
        state: State<'_, SharedState>,
    ) -> Result<Done<rekindle_types::dto::servers::RefreshAnswer>, CommandError> {
        IpcRequest::Servers(ServersRequest::Refresh)
    }
}

host_command! {
    state;
    /// Our time in each detected game, the most recent first
    /// (`IpcRequest::GameTotalsGet`, plan E7.2).
    pub async fn get_game_totals(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<GameTotalView>>, CommandError> {
        IpcRequest::GameTotalsGet
    }
}

host_command! {
    state;
    /// A friend's time in each game, as they last shared it
    /// (`IpcRequest::FriendGameTotalsGet`, plan E7.2); empty when they share
    /// none.
    pub async fn get_friend_game_totals(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<GameTotalView>>, CommandError> {
        IpcRequest::FriendGameTotalsGet { public_key }
    }
}

host_command! {
    state;
    /// The games hidden from everyone (`IpcRequest::HiddenGamesGet`, plan E7.2).
    pub async fn get_hidden_games(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<HiddenGameView>>, CommandError> {
        IpcRequest::HiddenGamesGet
    }
}

host_command! {
    state;
    /// Hide a game from everyone (`IpcRequest::GameHide`, plan E7.2): the hide
    /// list comes back.
    pub async fn hide_game(
        game_id: u32,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<HiddenGameView>>, CommandError> {
        IpcRequest::GameHide { game_id }
    }
}

host_command! {
    state;
    /// Take a game off the hide list (`IpcRequest::GameUnhide`, plan E7.2, a
    /// trust-changing intent the Host reviews): the hide list comes back.
    pub async fn unhide_game(
        game_id: u32,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<HiddenGameView>>, CommandError> {
        IpcRequest::GameUnhide { game_id }
    }
}
