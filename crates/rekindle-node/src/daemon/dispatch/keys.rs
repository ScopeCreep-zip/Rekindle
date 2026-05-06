//! Key management dispatch handlers: MekList, MekRotate, MekRequest, PrekeyReplenish.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use super::{DaemonContext, state_error};

pub(crate) fn handle_mek_list(
    ctx: &DaemonContext, state: DaemonState, community: &str,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let snapshot = ctx.mek_cache.read().snapshot(community);
    IpcResponse::ok(&snapshot)
}

pub(crate) async fn handle_mek_rotate(
    ctx: &DaemonContext, state: DaemonState, community: &str, channel: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };

    match rekindle_transport::operations::mek::rotate_mek(
        &transport, &membership, channel, &ctx.mek_cache, &signing_key,
    ).await {
        Ok(result) => IpcResponse::ok(&serde_json::json!({
            "rotated": true,
            "community": community,
            "channel": channel,
            "generation": result.generation,
            "copies_written": result.copies_written,
        })),
        Err(e) => IpcResponse::error(500, format!("mek rotate failed: {e}")),
    }
}

pub(crate) fn handle_mek_request(
    ctx: &DaemonContext, state: DaemonState,
    community: &str, channel: &str, generation: u64,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };

    match rekindle_transport::operations::mek::build_mek_request_payload(
        channel, generation, &membership.pseudonym_key,
    ) {
        Ok(payload_bytes) => IpcResponse::ok(&serde_json::json!({
            "gossip_payload_len": payload_bytes.len(),
            "channel": channel,
            "generation": generation,
        })),
        Err(e) => IpcResponse::error(500, format!("mek request: {e}")),
    }
}

pub(crate) async fn handle_prekey_replenish(
    ctx: &DaemonContext, state: DaemonState,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let profile_key = match ctx.require_session(|s| s.identity.profile_dht_key.clone()) {
        Ok(k) => k, Err(e) => return e,
    };

    match rekindle_transport::operations::mek::replenish_prekeys(
        &transport, &profile_key, &signing_key,
    ).await {
        Ok(count) => IpcResponse::ok(&serde_json::json!({
            "replenished": true,
            "bytes_written": count,
        })),
        Err(e) => IpcResponse::error(500, format!("prekey replenish: {e}")),
    }
}
