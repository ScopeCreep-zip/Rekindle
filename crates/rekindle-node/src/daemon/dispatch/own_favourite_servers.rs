//! Our favourite game servers (plan E7.2), on both hosts: stored locally in
//! `favourite_servers`, never sent. Each view carries `invitable`, the
//! Host's answer to whether a `GameInvite` may name it (the game is not
//! hidden and current-game sharing is on), so no frontend decides it.

use rekindle_db::repo::favourite_servers::{self, FavouriteServer};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::presence::FavouriteServerView;

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Route one of the favourite-server requests (`own::route`'s arm).
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
) -> IpcResponse {
    let write = !matches!(request, IpcRequest::FavouriteServersGet);
    if write && !state.can_write() {
        return state_error(state, "write");
    }
    if !write && !state.can_query() {
        return state_error(state, "query");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let now_ms = ctx.clock.now_ms_i64();
    let added_at = now_ms / 1000;
    // Our own feed entry (plan E7.15), stored once the add is.
    let added = match &request {
        IpcRequest::FavouriteServerAdd {
            game_id,
            server,
            label,
        } => Some(rekindle_presence::activity::favourite_added(
            *game_id,
            server.as_str(),
            label.trim(),
            now_ms,
        )),
        _ => None,
    };
    let read = db
        .call(move |conn| {
            match request {
                IpcRequest::FavouriteServerAdd {
                    game_id,
                    server,
                    label,
                } => favourite_servers::add(
                    conn,
                    &owner,
                    &FavouriteServer {
                        game_id,
                        address: server.as_str().to_owned(),
                        label: label.trim().to_owned(),
                        added_at,
                    },
                )?,
                IpcRequest::FavouriteServerRemove { game_id, server } => {
                    favourite_servers::remove(conn, &owner, game_id, server.as_str())?;
                }
                _ => {}
            }
            let shared = rekindle_db::repo::preferences::load(conn, &owner)?.share_current_game;
            let hidden = rekindle_db::repo::hidden_games::list(conn, &owner)?;
            let stored = favourite_servers::list(conn, &owner)?;
            Ok(stored
                .into_iter()
                .map(|f| {
                    let invitable = shared && !hidden.contains(&f.game_id);
                    view(f, invitable)
                })
                .collect::<Vec<_>>())
        })
        .await;
    match read {
        Ok(views) => {
            if let Some(entry) = added {
                crate::host::activity::own(ctx, entry).await;
            }
            IpcResponse::ok(&views)
        }
        Err(e) => IpcResponse::error(500, format!("favourite servers: {e}")),
    }
}

fn view(f: FavouriteServer, invitable: bool) -> FavouriteServerView {
    let entry = rekindle_game_detect::GameDatabase::shared().lookup_by_id(f.game_id);
    FavouriteServerView {
        game_id: f.game_id,
        game_name: entry.map_or_else(|| format!("Game {}", f.game_id), |e| e.name.clone()),
        icon_key: entry.and_then(rekindle_game_detect::database::GameEntry::icon_key),
        address: f.address,
        label: f.label,
        added_at: f.added_at,
        invitable,
    }
}
