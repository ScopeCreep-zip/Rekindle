//! The one game-launch path (lead decision (50), plan E7.15 IV3): the games
//! we can launch, and `GameLaunch`, which every frontend sends. The intent
//! gate has already classed the launch by its `JoinSource`
//! (`host::intent_select::game_launch`); `validation` has checked that the
//! game has a launch method for it.

use rekindle_game_detect::{installed, launcher, GameDatabase};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::presence::InstalledGameView;
use rekindle_types::game_server::ServerAddress;

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Route `InstalledGamesGet` or `GameLaunch` (`own::route`'s arm for them).
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
) -> IpcResponse {
    match request {
        IpcRequest::InstalledGamesGet => {
            if !state.can_query() {
                return state_error(state, "query");
            }
            installed_games(ctx).await
        }
        IpcRequest::GameLaunch {
            game_id, server, ..
        } => {
            if !state.can_write() {
                return state_error(state, "write");
            }
            launch(game_id, server).await
        }
        other => unreachable!("{} is not a game launch request", other.name()),
    }
}

/// `InstalledGamesGet`: Steam's installed games (read only while game
/// detection is on, threat model I10) with the games we have played,
/// minus the hide list, named from the bundled database.
async fn installed_games(ctx: &DaemonContext) -> IpcResponse {
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let read = db
        .call(move |conn| {
            let prefs = rekindle_db::repo::preferences::load(conn, &owner)?;
            let played = rekindle_db::repo::game_sessions::totals(conn, &owner)?;
            let hidden = rekindle_db::repo::hidden_games::list(conn, &owner)?;
            Ok((prefs.game_detection_enabled, played, hidden))
        })
        .await;
    let (detection, played, hidden) = match read {
        Ok(read) => read,
        Err(e) => return IpcResponse::error(500, format!("games not read: {e}")),
    };
    let steam = if detection {
        match rekindle_lifecycle::run_blocking(installed::scan_steam).await {
            Ok(ids) => ids,
            Err(e) => return IpcResponse::error(500, format!("Steam library not read: {e}")),
        }
    } else {
        std::collections::BTreeSet::new()
    };
    let played: Vec<u32> = played.iter().map(|total| total.game_id).collect();
    let database = GameDatabase::shared();
    let games: Vec<InstalledGameView> =
        installed::installed_games(database, &steam, &played, &hidden)
            .into_iter()
            .map(|game_id| view(database, game_id))
            .collect();
    IpcResponse::ok(&games)
}

fn view(database: &GameDatabase, game_id: u32) -> InstalledGameView {
    let entry = database.lookup_by_id(game_id);
    InstalledGameView {
        game_id,
        game_name: entry.map_or_else(|| format!("Game {game_id}"), |entry| entry.name.clone()),
        icon_key: entry.and_then(rekindle_game_detect::database::GameEntry::icon_key),
        launchable: entry.is_some_and(launcher::launchable),
    }
}

/// `GameLaunch`, once the gate let it run: the OS opens the launch URI.
async fn launch(game_id: u32, server: Option<ServerAddress>) -> IpcResponse {
    let opened = rekindle_lifecycle::run_blocking(move || {
        launcher::launch(GameDatabase::shared(), game_id, server.as_ref())
    })
    .await;
    match opened {
        Ok(Ok(())) => IpcResponse::ok(&()),
        Ok(Err(e)) => IpcResponse::error(500, format!("game not launched: {e}")),
        Err(e) => IpcResponse::error(500, format!("game not launched: {e}")),
    }
}
