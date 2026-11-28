//! Auto-away's threshold (plan E7.4), one implementation on both hosts:
//! saved, then applied at once over `host::idle`, so an auto-away whose
//! reason went (off, or a threshold above our idle time) ends now, not at
//! the idle stage's next tick (Xfire cleared auto-away as soon as its
//! reason went; a status the user picked stays).

use rekindle_ipc::protocol::IpcResponse;

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// The threshold's bound: the schema's `CHECK(auto_away_minutes BETWEEN 0
/// AND 1440)`.
const MAX_MINUTES: u32 = 1440;

/// `SetAutoAway`: saved, then applied to our presence at once. The
/// preferences as they now are.
pub(super) async fn handle(ctx: &DaemonContext, state: DaemonState, minutes: u32) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if minutes > MAX_MINUTES {
        return IpcResponse::error(
            400,
            format!("invalid auto-away minutes: {minutes} (at most {MAX_MINUTES})"),
        );
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::preferences::set_auto_away_minutes(conn, &owner, minutes)?;
            rekindle_db::repo::preferences::load(conn, &owner)
        })
        .await;
    match saved {
        Ok(prefs) => {
            crate::host::idle::apply_now(&ctx.own_presence, &ctx.idle, minutes).await;
            IpcResponse::ok(&prefs)
        }
        Err(e) => IpcResponse::error(500, format!("auto-away not saved: {e}")),
    }
}
