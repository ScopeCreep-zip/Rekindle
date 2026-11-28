//! The local activity feed's requests (plan E7.15, IV2): the
//! `keep_activity_log` setting and the `ActivityList` page. One
//! implementation on both hosts, over `host::activity`.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Route one of the activity feed's requests (`own::route`'s arm for them).
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
) -> IpcResponse {
    match request {
        IpcRequest::SetKeepActivityLog { enabled } => handle_keep(ctx, state, enabled).await,
        IpcRequest::ActivityList { before, limit } => {
            if !state.can_query() {
                return state_error(state, "query");
            }
            list(ctx, before, limit).await
        }
        other => unreachable!("{} is not an activity feed request", other.name()),
    }
}

/// `SetKeepActivityLog`: saved, and off clears the feed in the same
/// transaction. The preferences as they now are.
async fn handle_keep(ctx: &DaemonContext, state: DaemonState, enabled: bool) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::preferences::set_keep_activity_log(conn, &owner, enabled)?;
            rekindle_db::repo::preferences::load(conn, &owner)
        })
        .await;
    match saved {
        Ok(prefs) => IpcResponse::ok(&prefs),
        Err(e) => IpcResponse::error(500, format!("activity log setting not saved: {e}")),
    }
}

/// `ActivityList`: one page, newest first, names resolved now.
async fn list(ctx: &DaemonContext, before: Option<i64>, limit: u16) -> IpcResponse {
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let page = db
        .call(move |conn| rekindle_db::repo::activity::page(conn, &owner, before, limit))
        .await;
    match page {
        Ok(rows) => IpcResponse::ok(&crate::host::activity::items(ctx, rows)),
        Err(e) => IpcResponse::error(500, format!("activity feed not read: {e}")),
    }
}
