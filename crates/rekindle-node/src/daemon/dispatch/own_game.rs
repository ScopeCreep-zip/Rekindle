//! Our game privacy settings (plan E7.2, prod research batch 7): detection
//! (local only) split from showing the current game to friends, and the
//! hide list. One implementation on both hosts, over `host::game_stage`.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::presence::HiddenGameView;

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Route one of the game privacy requests (`own::route`'s arm for them).
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
) -> IpcResponse {
    match request {
        IpcRequest::SetGameDetection { enabled } => handle_detection(ctx, state, enabled).await,
        IpcRequest::SetShareCurrentGame { enabled } => handle_share(ctx, state, enabled).await,
        IpcRequest::HiddenGamesGet => {
            if !state.can_query() {
                return state_error(state, "query");
            }
            hidden_games(ctx).await
        }
        IpcRequest::GameHide { game_id } => handle_hidden(ctx, state, game_id, true).await,
        IpcRequest::GameUnhide { game_id } => handle_hidden(ctx, state, game_id, false).await,
        other => unreachable!("{} is not a game privacy request", other.name()),
    }
}

/// `SetGameDetection`: saved, then the scanner starts or stops at once
/// (off, the process table is never read). The preferences as they now are.
async fn handle_detection(ctx: &DaemonContext, state: DaemonState, enabled: bool) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let write = owner.clone();
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::preferences::set_game_detection_enabled(conn, &write, enabled)?;
            rekindle_db::repo::preferences::load(conn, &write)
        })
        .await;
    let prefs = match saved {
        Ok(prefs) => prefs,
        Err(e) => return IpcResponse::error(500, format!("game detection not saved: {e}")),
    };
    if enabled {
        if let Err(e) = ctx.game.start(ctx, &ctx.unlock_scope_or_closed(), owner) {
            return IpcResponse::error(503, format!("game detection not started: {e}"));
        }
    } else {
        ctx.game.stop().await;
    }
    IpcResponse::ok(&prefs)
}

/// `SetShareCurrentGame`, once its review was committed (turning it on is
/// TrustChanging): saved, then the detected game is shown or cleared at
/// once. The preferences as they now are.
async fn handle_share(ctx: &DaemonContext, state: DaemonState, enabled: bool) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::preferences::set_share_current_game(conn, &owner, enabled)?;
            rekindle_db::repo::preferences::load(conn, &owner)
        })
        .await;
    match saved {
        Ok(prefs) => {
            ctx.game.reapply().await;
            IpcResponse::ok(&prefs)
        }
        Err(e) => IpcResponse::error(500, format!("game sharing not saved: {e}")),
    }
}

/// `GameHide` / `GameUnhide`: the list changes, the shown game follows it,
/// and our totals go again (only while totals sharing is on) so friends'
/// copies, which each frame replaces, drop a newly hidden game. The hide
/// list as it now is.
async fn handle_hidden(
    ctx: &DaemonContext,
    state: DaemonState,
    game_id: u32,
    hidden: bool,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let written = db
        .call(move |conn| {
            if hidden {
                rekindle_db::repo::hidden_games::hide(conn, &owner, game_id)
            } else {
                rekindle_db::repo::hidden_games::unhide(conn, &owner, game_id)
            }
        })
        .await;
    if let Err(e) = written {
        return IpcResponse::error(500, format!("hide list not saved: {e}"));
    }
    ctx.game.reapply().await;
    let friends = crate::host::adapters::friends::HostFriends::of(ctx);
    let reached =
        rekindle_transport::operations::friend::game_totals::send_to_friends(&friends).await;
    tracing::debug!(reached, "game totals sent after the hide list changed");
    hidden_games(ctx).await
}

/// The hide list, named and given icon keys from the bundled database.
async fn hidden_games(ctx: &DaemonContext) -> IpcResponse {
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    match db
        .call(move |conn| rekindle_db::repo::hidden_games::list(conn, &owner))
        .await
    {
        Ok(ids) => IpcResponse::ok(&ids.into_iter().map(view).collect::<Vec<_>>()),
        Err(e) => IpcResponse::error(500, format!("hide list not read: {e}")),
    }
}

fn view(game_id: u32) -> HiddenGameView {
    let entry = rekindle_game_detect::GameDatabase::shared().lookup_by_id(game_id);
    HiddenGameView {
        game_id,
        game_name: entry.map_or_else(|| format!("Game {game_id}"), |entry| entry.name.clone()),
        icon_key: entry.and_then(rekindle_game_detect::database::GameEntry::icon_key),
    }
}
