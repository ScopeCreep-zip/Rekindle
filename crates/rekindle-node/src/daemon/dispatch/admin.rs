//! Admin dispatch handlers: Agent*, Policy, Network*.
//!
//! Subscribe/Unsubscribe are handled server-side in the IPC bus router
//! via EventRouter — they never reach daemon dispatch.

use crate::daemon::DaemonState;
use rekindle_ipc::message::{AgentType, SecurityLevel};
use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::server::DAEMON_AGENT_NAME;

use super::{state_error, CallerContext, DaemonContext};

// ── Network ─────────────────────────────────────────────────────────────

/// Handle NetworkStatus — detailed transport node status.
pub(crate) fn handle_network_status(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport_guard = ctx.transport.read();
    let Some(ref transport) = *transport_guard else {
        return IpcResponse::error(503, "transport not started");
    };

    let snap = transport.status_snapshot();
    let peer_reg = transport.peers();
    let peers = peer_reg.read();
    let circuit = peers.circuit_summary();

    IpcResponse::ok(&serde_json::json!({
        "attachment": snap.attachment,
        "is_attached": snap.is_attached,
        "public_internet_ready": snap.public_internet_ready,
        "uptime_secs": snap.uptime_secs,
        "peer_count": snap.peer_count,
        "route_allocated": snap.route_allocated,
        "route_age_secs": snap.route_age_secs,
        "circuit_summary": {
            "total": circuit.total,
            "healthy": circuit.healthy,
            "degraded": circuit.degraded,
            "circuit_open": circuit.circuit_open,
        },
    }))
}

/// Handle NetworkPeers — peer snapshot for display.
pub(crate) fn handle_network_peers(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport_guard = ctx.transport.read();
    let Some(ref transport) = *transport_guard else {
        return IpcResponse::error(503, "transport not started");
    };

    let peers = transport.peers();
    let snapshot = peers.read().snapshot();
    IpcResponse::ok(&snapshot)
}

// ── Agent Management ────────────────────────────────────────────────────

/// Handle AgentRegister — register the calling connection under `name`.
///
/// The registry key is the caller's Noise static key, which the bus server
/// stamped from the handshake (`CallerContext::static_key`), so an agent
/// can only ever register itself. The registration takes effect on the
/// agent's next connection, whose handshake looks the key up. Clearance
/// stays `Open`; raising it is the D1 clearance gate.
pub(crate) async fn handle_agent_register(
    ctx: &DaemonContext,
    caller: &CallerContext,
    name: &str,
    agent_type: AgentType,
    capabilities: &[String],
) -> IpcResponse {
    let Some(static_key) = caller.static_key else {
        return IpcResponse::error(500, "the bus server did not stamp the caller's key");
    };
    // Dispatch runs on the runtime, so the registry lock must be awaited
    // — a blocking lock here panics the tokio worker.
    let mut registry = ctx.registry.write().await;
    if registry.find_by_name(name).is_some() {
        return IpcResponse::error(409, format!("agent '{name}' is already registered"));
    }
    if let Some(existing) = registry.lookup_name(&static_key) {
        return IpcResponse::error(
            409,
            format!("this connection's key is already registered as '{existing}'"),
        );
    }
    registry.register(
        name.to_owned(),
        static_key,
        SecurityLevel::Open,
        agent_type,
        capabilities.to_vec(),
    );
    drop(registry);
    tracing::info!(agent = name, ?agent_type, "agent registered");

    IpcResponse::ok(&serde_json::json!({
        "registered": true,
        "name": name,
        "agent_type": format!("{agent_type:?}"),
        "capabilities": capabilities,
    }))
}

/// Handle AgentRevoke — remove an agent from the ClearanceRegistry. The
/// daemon's own registration is not revocable: it is how the bus routes
/// every request.
pub(crate) async fn handle_agent_revoke(ctx: &DaemonContext, name: &str) -> IpcResponse {
    if name == DAEMON_AGENT_NAME {
        return IpcResponse::error(409, "the daemon's own registration cannot be revoked");
    }
    let mut registry = ctx.registry.write().await;
    match registry.revoke_by_name(name) {
        Some(identity) => {
            tracing::info!(
                agent = name,
                generation = identity.generation,
                "agent revoked from registry"
            );
            IpcResponse::ok(&serde_json::json!({
                "revoked": true,
                "name": name,
                "generation": identity.generation,
            }))
        }
        None => IpcResponse::error(404, format!("agent '{name}' not found in registry")),
    }
}

// ── Policy ──────────────────────────────────────────────────────────────

/// Handle PolicyReload — reload authorization policy from disk through the
/// host's one loader (`host::policy::load_layered`): the system layer, then
/// the user layer, each only tightening. A layer that does not parse fails
/// the reload and leaves the active policy unchanged.
pub(crate) fn handle_policy_reload(ctx: &DaemonContext) -> IpcResponse {
    let policy = match crate::host::policy::load_layered() {
        Ok(policy) => policy,
        Err(e) => return IpcResponse::error(500, e.to_string()),
    };

    *ctx.policy.write() = policy.clone();

    IpcResponse::ok(&serde_json::json!({
        "reloaded": true,
        "min_hop_count": policy.min_hop_count,
        "max_gossip_ttl": policy.max_gossip_ttl,
    }))
}
