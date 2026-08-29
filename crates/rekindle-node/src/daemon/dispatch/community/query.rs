//! Community queries: list, info.

use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use crate::daemon::dispatch::{state_error, DaemonContext};

pub(crate) fn handle_list(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    ctx.require_session(|session| {
        let communities: Vec<serde_json::Value> = session
            .communities
            .values()
            .map(|m| {
                serde_json::json!({
                    "governance_key": m.governance_key,
                    "name": m.community_name,
                    "description": "",
                    "member_count": 0,
                    "channel_count": 0,
                    "our_pseudonym": m.pseudonym_key,
                })
            })
            .collect();
        IpcResponse::ok(&communities)
    })
    .unwrap_or_else(|e| e)
}

pub(crate) async fn handle_info(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    match query.community_detail(&membership).await {
        Ok(detail) => IpcResponse::ok(&detail),
        Err(e) => IpcResponse::error(500, format!("community detail: {e}")),
    }
}
