//! The Servers view (plan E7.2 G3, SK2-G design §3; Xfire's Servers tab):
//! `ServersView` composes it from what we hold, `RecentServerRemove`
//! forgets a visited server, and one worker pushes the view again
//! (`SystemEvent::ServersViewChanged`) when what it shows changes. The same
//! request path serves the desktop, the TUI and the CLI (decisions 51, 52).

mod compose;
mod refresh;
mod worker;

use std::collections::VecDeque;

use rekindle_db::repo::recent_servers::{self, RecentServer};
use rekindle_ipc::protocol::{IpcResponse, ServersRequest};
use rekindle_types::game_server::ServerAddress;
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use crate::daemon::dispatch::{state_error, DaemonContext};
use crate::daemon::DaemonState;

pub(crate) use compose::compose;
use worker::push;
pub(crate) use worker::run_worker;

/// How many received game invites the view keeps (newest first).
pub(crate) const MAX_INVITES: usize = 20;

/// A friend's game invite as the Friends category shows it: held in
/// memory for the session only, like Xfire's invites (never stored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeldInvite {
    pub peer_key: String,
    pub game_id: u32,
    pub server: String,
}

/// The session's received game invites (`ctx.servers`), the worker's
/// trigger for a change no journal event names (a favourite removed), and
/// the Refresh's port and answers (plan E7.2 G4).
#[derive(Debug)]
pub(crate) struct ServersState {
    invites: parking_lot::Mutex<VecDeque<HeldInvite>>,
    stale: tokio::sync::Notify,
    pub(crate) refresh: refresh::RefreshState,
}

impl ServersState {
    /// Over the Host's server-query port (`HostConfig::server_query`).
    pub(crate) fn new(
        query: std::sync::Arc<dyn rekindle_game_detect::server_query::ServerQuery>,
    ) -> Self {
        Self {
            invites: parking_lot::Mutex::default(),
            stale: tokio::sync::Notify::new(),
            refresh: refresh::RefreshState::new(query),
        }
    }

    /// Hold `invite` first; a repeat moves to the front, the oldest past
    /// [`MAX_INVITES`] goes.
    pub(crate) fn hold(&self, invite: HeldInvite) {
        let mut held = self.invites.lock();
        held.retain(|i| *i != invite);
        held.push_front(invite);
        held.truncate(MAX_INVITES);
    }

    pub(crate) fn invites(&self) -> Vec<HeldInvite> {
        self.invites.lock().iter().cloned().collect()
    }

    /// What the view shows changed without a journal event: the worker
    /// composes again one coalescing period later. A mark made while the
    /// worker is busy is kept (one permit), never lost.
    pub(crate) fn mark_stale(&self) {
        self.stale.notify_one();
    }

    /// Resolves at the next [`Self::mark_stale`].
    pub(super) async fn marked_stale(&self) {
        self.stale.notified().await;
    }

    /// The session ended.
    pub(crate) fn clear(&self) {
        self.invites.lock().clear();
        self.refresh.clear();
    }
}

/// Route `ServersView` (Query) or `RecentServerRemove` (Write), which
/// answer the view; the favourites trio answers its own list.
pub(crate) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ServersRequest,
) -> IpcResponse {
    match request {
        ServersRequest::Refresh => return refresh::start(ctx, state).await,
        request @ (ServersRequest::FavouritesGet
        | ServersRequest::FavouriteAdd { .. }
        | ServersRequest::FavouriteRemove { .. }) => {
            return crate::daemon::dispatch::own_favourite_servers::route(ctx, state, request)
                .await;
        }
        ServersRequest::View => {
            if !state.can_query() {
                return state_error(state, "query");
            }
        }
        ServersRequest::RecentRemove { game_id, server } => {
            if !state.can_write() {
                return state_error(state, "write");
            }
            let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
                (Ok(db), Ok(owner)) => (db, owner),
                (Err(e), _) | (_, Err(e)) => return e,
            };
            let removed = db
                .call(move |conn| {
                    recent_servers::remove(conn, &owner, game_id, server.as_str())?;
                    Ok(())
                })
                .await;
            if let Err(e) = removed {
                return IpcResponse::error(500, format!("recent server not removed: {e}"));
            }
            return answer_and_push(ctx).await;
        }
    }
    match compose(ctx).await {
        Ok(view) => IpcResponse::ok(&view),
        Err(e) => e,
    }
}

/// Compose the view, push it to every window showing it, and answer it.
async fn answer_and_push(ctx: &DaemonContext) -> IpcResponse {
    match compose(ctx).await {
        Ok(view) => {
            let answer = IpcResponse::ok(&view);
            ctx.publish_event(SubscriptionEvent::System(SystemEvent::ServersViewChanged {
                view,
            }));
            answer
        }
        Err(e) => e,
    }
}

/// We visited `server` for `game_id` (our presence shows it, or we launched
/// to it): record it in `recent_servers` and push the view.
pub(crate) async fn visited(ctx: &DaemonContext, game_id: u32, server: &ServerAddress) {
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return;
    };
    let recent = RecentServer {
        game_id,
        address: server.as_str().to_owned(),
        last_seen: ctx.clock.now_ms_i64() / 1000,
    };
    let stored = db
        .call(move |conn| {
            recent_servers::touch(conn, &owner, &recent)?;
            Ok(())
        })
        .await;
    match stored {
        Ok(()) => {
            answer_and_push(ctx).await;
        }
        Err(e) => tracing::warn!(game_id, ?e, "recent server not stored"),
    }
}
