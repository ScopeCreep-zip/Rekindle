//! Peer route and route-import accessors.
//!
//! A peer's latest route blob lives in one place, the transport's
//! `PeerRegistry` (plan E2.1). Turning a blob into a `RouteId` is the
//! process's one importer, `AppState.route_imports` (plan C7.6), which the
//! transport's route port shares: it has no TTL and releases a route only
//! after a send to it failed.

use std::sync::Arc;

use crate::state::AppState;

use super::node::safe_api_and_routing_context;

/// A peer's route blob from the transport's one peer-route cache
/// (`PeerRegistry`, plan E2.1), while it is fresh.
pub fn peer_route(state: &AppState, peer_key: &str) -> Option<Vec<u8>> {
    let node = state.transport.read().clone()?;
    rekindle_transport::operations::dm::route::cached(&node, peer_key)
}

/// The Host's friend operations, for the unported callers of the 1:1 send
/// path (`operations::dm`).
///
/// # Errors
/// The Host has not started.
pub fn friend_host(
    state: &AppState,
) -> Result<Arc<dyn rekindle_transport::operations::friend::FriendHost>, String> {
    state
        .host
        .current()
        .map(|host| host.friend_host())
        .ok_or_else(|| "the host has not started".to_string())
}

/// A peer's name for an OS notice: our nickname for them, else their
/// key's fingerprint (the transport's `friend::label`), never the name
/// their own payload carried. With no Host there is no nickname to hold.
pub fn peer_label(state: &AppState, peer_key: &str) -> String {
    use rekindle_transport::operations::friend;
    friend_host(state).map_or_else(
        |_| friend::short_key(peer_key),
        |host| friend::label(host.as_ref(), peer_key),
    )
}

/// The `RouteId` for a route blob, through the process's one importer.
pub fn import_route_blob(
    state: &Arc<AppState>,
    route_blob: &[u8],
) -> Result<veilid_core::RouteId, String> {
    route_imports(state)?
        .get_or_import(route_blob)
        .map_err(|e| e.to_string())
}

/// A send to `route_id` showed the peer's route unusable
/// (`rekindle_protocol::ports::peer_route_unusable`): the importer forgets
/// it (never releases: Veilid releases a dead remote route itself, plan
/// C7.9e).
pub fn route_send_failed(state: &Arc<AppState>, route_id: &veilid_core::RouteId) {
    if let Ok(imports) = route_imports(state) {
        imports.invalidate_after_send_failure(route_id);
    }
}

/// Veilid declared these imported routes dead (`RouteChange`): the
/// transport's decision (`TransportNode::on_dead_remote_routes`, shared
/// with the daemon's dispatch). Returns the peers whose blob was one.
pub fn on_dead_remote_routes(state: &AppState, dead: &[String]) -> Vec<String> {
    state
        .transport
        .read()
        .as_ref()
        .map(|node| node.on_dead_remote_routes(dead))
        .unwrap_or_default()
}

/// `app_call` a peer through its route blob: import (through the one
/// importer), call on the safe routing context within the transport's
/// finding 78 bound (`rekindle_transport::broadcast::deadline`: Veilid
/// bounds only the answer wait, never the stages before it), and forget
/// the route if the call shows it unusable ([`route_send_failed`]). A call
/// at the bound is a timeout, outcome unknown: the route stays.
pub async fn call_route_blob(
    state: &Arc<AppState>,
    route_blob: &[u8],
    payload: Vec<u8>,
) -> Result<Vec<u8>, String> {
    use rekindle_transport::broadcast::deadline::app_call_deadline;
    let route_id = import_route_blob(state, route_blob)?;
    let (_, rc) = safe_api_and_routing_context(state).ok_or("not attached")?;
    let deadline = app_call_deadline(&safe_profile());
    let call = rc.app_call(veilid_core::Target::RouteId(route_id.clone()), payload);
    let result = tokio::time::timeout(deadline, call)
        .await
        .map_err(|_| unresolved("app_call", deadline))?;
    settle_route(state, &route_id, result).map_err(|e| format!("app_call: {e}"))
}

/// `app_message` a peer through its route blob; as [`call_route_blob`],
/// within the transport's send bound (`app_message_deadline`, LV28).
pub async fn message_route_blob(
    state: &Arc<AppState>,
    route_blob: &[u8],
    payload: Vec<u8>,
) -> Result<(), String> {
    use rekindle_transport::broadcast::deadline::app_message_deadline;
    let route_id = import_route_blob(state, route_blob)?;
    let (_, rc) = safe_api_and_routing_context(state).ok_or("not attached")?;
    let deadline = app_message_deadline();
    let send = rc.app_message(veilid_core::Target::RouteId(route_id.clone()), payload);
    let result = tokio::time::timeout(deadline, send)
        .await
        .map_err(|_| unresolved("app_message", deadline))?;
    settle_route(state, &route_id, result).map_err(|e| format!("app_message: {e}"))
}

/// The profile of [`safe_api_and_routing_context`]'s routing context.
fn safe_profile() -> rekindle_types::config::SafetyProfile {
    rekindle_route::contexts::RouteContextSpec::rc_safe().safety_profile()
}

/// The error of a send still unresolved at our `deadline`.
fn unresolved(op: &str, deadline: std::time::Duration) -> String {
    tracing::warn!(
        op,
        deadline_ms = deadline.as_millis(),
        "route-blob send unresolved at our deadline: timed out, outcome unknown"
    );
    format!(
        "{op}: timed out: unresolved after {} ms (outcome unknown)",
        deadline.as_millis()
    )
}

/// Forget `route_id` when a send to it shows the **peer's** route
/// unusable; a `NoConnection` or `InvalidTarget` from our own safety route
/// or node keeps it (plan VU5, synthesis #9).
fn settle_route<T>(
    state: &Arc<AppState>,
    route_id: &veilid_core::RouteId,
    result: Result<T, veilid_core::VeilidAPIError>,
) -> Result<T, veilid_core::VeilidAPIError> {
    if let Err(e) = &result {
        if rekindle_protocol::ports::peer_route_unusable(e) {
            route_send_failed(state, route_id);
        }
    }
    result
}

/// The process's route importer (boot-scoped).
pub fn route_imports(
    state: &Arc<AppState>,
) -> Result<Arc<rekindle_protocol::dht::route_imports::RouteImports>, String> {
    state
        .route_imports
        .read()
        .clone()
        .ok_or_else(|| "Veilid not connected".to_string())
}
