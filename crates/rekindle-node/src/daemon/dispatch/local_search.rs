//! `LocalSearch` (plan E7.2 G11, SK2-G §8): what we already hold, by the
//! view's scope, read from the identity's database only
//! (`rekindle_db::repo::search::local_search`); no scope reaches the
//! network, since there is no directory to ask.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::local_search::{LocalSearchResult, LocalSearchScope};

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    scope: LocalSearchScope,
    query: String,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    match db
        .call(move |conn| rekindle_db::repo::search::local_search(conn, &owner, &scope, &query))
        .await
    {
        Ok(hits) => IpcResponse::ok(&LocalSearchResult { hits }),
        Err(e) => IpcResponse::error(500, format!("local search failed: {e}")),
    }
}
