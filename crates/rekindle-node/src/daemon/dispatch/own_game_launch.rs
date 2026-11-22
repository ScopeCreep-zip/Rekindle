//! The one game-launch path (lead decision (50), plan E7.15 IV3): the games
//! we can launch, and `GameLaunch`, which every frontend sends. The intent
//! gate has already classed the launch by its `JoinSource`
//! (`host::intent_select::game_launch`); `validation` has checked that the
//! game has a launch method for it.

use rekindle_game_detect::installed::SteamLayout;
use rekindle_game_detect::{installed, launcher, GameDatabase};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::presence::InstalledGameView;
use rekindle_types::game_server::ServerAddress;
use rekindle_utils::paths::AppRoots;

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
            let answer = launch(ctx.root.apps.clone(), game_id, server.clone()).await;
            // A launch to a server is a visit (plan E7.2 G3: `recent_servers`).
            if let (true, Some(server)) = (matches!(answer, IpcResponse::Ok { .. }), server) {
                crate::host::servers::visited(ctx, game_id, &server).await;
            }
            answer
        }
        other => unreachable!("{} is not a game launch request", other.name()),
    }
}

/// `InstalledGamesGet`: Steam's installed games and the OS's own native
/// games present (both read only while game
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
    let apps = ctx.root.apps.clone();
    let read = rekindle_lifecycle::run_blocking(move || {
        let database = GameDatabase::shared();
        let (steam, native) = if detection {
            (
                installed::scan_steam(&apps),
                installed::native_installed(database, SteamLayout::CURRENT, &apps),
            )
        } else {
            Default::default()
        };
        let played: Vec<u32> = played.iter().map(|total| total.game_id).collect();
        installed::installed_games(database, &steam, &native, &played, &hidden)
            .into_iter()
            .map(|game_id| view(database, &apps, game_id))
            .collect::<Vec<InstalledGameView>>()
    })
    .await;
    match read {
        Ok(games) => IpcResponse::ok(&games),
        Err(e) => IpcResponse::error(500, format!("installed games not read: {e}")),
    }
}

fn view(database: &GameDatabase, apps: &AppRoots, game_id: u32) -> InstalledGameView {
    let entry = database.lookup_by_id(game_id);
    InstalledGameView {
        game_id,
        game_name: entry.map_or_else(|| format!("Game {game_id}"), |entry| entry.name.clone()),
        icon_key: entry.and_then(rekindle_game_detect::database::GameEntry::icon_key),
        launchable: entry
            .is_some_and(|entry| launcher::launchable(entry, SteamLayout::CURRENT, apps)),
    }
}

/// `GameLaunch`, once the gate let it run: the OS opens the launch URI or
/// the native app, read under the Host's own roots.
async fn launch(apps: AppRoots, game_id: u32, server: Option<ServerAddress>) -> IpcResponse {
    let opened = rekindle_lifecycle::run_blocking(move || {
        launcher::launch(GameDatabase::shared(), &apps, game_id, server.as_ref())
    })
    .await;
    match opened {
        Ok(Ok(())) => IpcResponse::ok(&()),
        Ok(Err(e)) => IpcResponse::error(500, format!("game not launched: {e}")),
        Err(e) => IpcResponse::error(500, format!("game not launched: {e}")),
    }
}
