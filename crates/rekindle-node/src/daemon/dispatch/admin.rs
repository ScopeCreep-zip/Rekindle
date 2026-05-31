//! Admin dispatch handlers: Agent*, Policy, Network*.
//!
//! Subscribe/Unsubscribe are handled server-side in the IPC bus router
//! via EventRouter — they never reach daemon dispatch.

use crate::daemon::DaemonState;
use crate::ipc::message::AgentType;
use crate::ipc::noise_keys::validate_agent_name;
use crate::ipc::protocol::IpcResponse;

use super::{state_error, DaemonContext, PolicyConfig};

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

/// Handle AgentRegister — register a named agent in the ClearanceRegistry.
///
/// Validates the agent name (path-traversal safe), then inserts into the
/// shared registry with the declared capabilities. The agent's Noise IK
/// static pubkey is used as the registry key — this is extracted from the
/// connection state by the server layer and will be wired through when
/// dispatch receives connection context.
///
/// For now, we register with a zero pubkey placeholder. The server layer
/// should call `registry.register()` with the real pubkey after dispatch
/// returns success.
pub(crate) fn handle_agent_register(
    ctx: &DaemonContext,
    name: &str,
    agent_type: AgentType,
    capabilities: &[String],
) -> IpcResponse {
    if let Err(e) = validate_agent_name(name) {
        return IpcResponse::error(400, format!("invalid agent name: {e}"));
    }

    // Check if name is already registered
    let registry = ctx.registry.blocking_read();
    if registry.find_by_name(name).is_some() {
        return IpcResponse::error(409, format!("agent '{name}' is already registered"));
    }
    drop(registry);

    // The actual pubkey-keyed registration happens in the server layer
    // after this response is sent back. We validate and ack here.
    IpcResponse::ok(&serde_json::json!({
        "registered": true,
        "name": name,
        "agent_type": format!("{agent_type:?}"),
        "capabilities": capabilities,
    }))
}

/// Handle AgentRevoke — remove an agent from the ClearanceRegistry.
pub(crate) fn handle_agent_revoke(ctx: &DaemonContext, name: &str) -> IpcResponse {
    if let Err(e) = validate_agent_name(name) {
        return IpcResponse::error(400, format!("invalid agent name: {e}"));
    }

    let mut registry = ctx.registry.blocking_write();
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

/// Handle PolicyReload — reload authorization policy from disk.
///
/// Loads policy from two paths in order:
/// 1. `/etc/rekindle/policy.toml` (system-wide, set by admin)
/// 2. `~/.config/rekindle/policy.toml` (user-level override)
///
/// System policy fields override user policy (admin constraints are
/// additive and cannot be weakened by user config).
pub(crate) fn handle_policy_reload(ctx: &DaemonContext) -> IpcResponse {
    let system_path = std::path::Path::new("/etc/rekindle/policy.toml");
    let user_path = ctx.config_dir.join("policy.toml");

    let mut policy = PolicyConfig::default();

    // Load system policy first (admin authority)
    if system_path.exists() {
        match load_policy_file(system_path) {
            Ok(sys) => {
                merge_policy(&mut policy, &sys);
                tracing::info!("system policy loaded from {}", system_path.display());
            }
            Err(e) => {
                return IpcResponse::error(
                    500,
                    format!(
                        "system policy parse failed ({}): {e}",
                        system_path.display()
                    ),
                );
            }
        }
    }

    // Load user policy (cannot weaken system policy)
    if user_path.exists() {
        match load_policy_file(&user_path) {
            Ok(usr) => {
                merge_policy(&mut policy, &usr);
                tracing::info!("user policy loaded from {}", user_path.display());
            }
            Err(e) => {
                return IpcResponse::error(
                    500,
                    format!("user policy parse failed ({}): {e}", user_path.display()),
                );
            }
        }
    }

    *ctx.policy.write() = policy.clone();

    IpcResponse::ok(&serde_json::json!({
        "reloaded": true,
        "min_hop_count": policy.min_hop_count,
        "require_signature_verification": policy.require_signature_verification,
        "max_gossip_ttl": policy.max_gossip_ttl,
    }))
}

fn load_policy_file(path: &std::path::Path) -> Result<PolicyConfig, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    toml::from_str(&content).map_err(|e| format!("parse failed: {e}"))
}

/// Merge a loaded policy layer into the active policy.
///
/// Constraint direction is one-way: each successive layer can only
/// tighten constraints, never relax them. This means:
///
/// - `/etc/rekindle/policy.toml` (system admin) establishes the floor.
/// - `~/.config/rekindle/policy.toml` (user) can raise the floor higher
///   (more hops, stricter verification, lower TTL) but can never lower it.
///
/// A user who sets `min_hop_count = 0` when the system requires `2` gets `2`.
/// A user who sets `min_hop_count = 4` when the system requires `2` gets `4`.
fn merge_policy(active: &mut PolicyConfig, loaded: &PolicyConfig) {
    // min_hop_count: higher value wins (more privacy, never less)
    match (active.min_hop_count, loaded.min_hop_count) {
        (Some(a), Some(b)) => active.min_hop_count = Some(a.max(b)),
        (None, Some(b)) => active.min_hop_count = Some(b),
        _ => {}
    }
    // require_signature_verification: once true, cannot be set false
    if loaded.require_signature_verification {
        active.require_signature_verification = true;
    }
    // max_gossip_ttl: lower value wins (more restrictive, never more)
    match (active.max_gossip_ttl, loaded.max_gossip_ttl) {
        (Some(a), Some(b)) => active.max_gossip_ttl = Some(a.min(b)),
        (None, Some(b)) => active.max_gossip_ttl = Some(b),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_policy_tightens_constraints() {
        let mut active = PolicyConfig {
            min_hop_count: Some(1),
            require_signature_verification: false,
            max_gossip_ttl: Some(5),
        };
        let loaded = PolicyConfig {
            min_hop_count: Some(2),
            require_signature_verification: true,
            max_gossip_ttl: Some(3),
        };
        merge_policy(&mut active, &loaded);
        assert_eq!(active.min_hop_count, Some(2)); // higher = more privacy
        assert!(active.require_signature_verification); // true wins
        assert_eq!(active.max_gossip_ttl, Some(3)); // lower = more restrictive
    }

    #[test]
    fn merge_policy_does_not_loosen() {
        let mut active = PolicyConfig {
            min_hop_count: Some(3),
            require_signature_verification: true,
            max_gossip_ttl: Some(2),
        };
        let loaded = PolicyConfig {
            min_hop_count: Some(1),
            require_signature_verification: false,
            max_gossip_ttl: Some(8),
        };
        merge_policy(&mut active, &loaded);
        assert_eq!(active.min_hop_count, Some(3)); // not lowered
        assert!(active.require_signature_verification); // not disabled
        assert_eq!(active.max_gossip_ttl, Some(2)); // not raised
    }

    // Subscribe/Unsubscribe tests moved to event_router.rs — server-side handling.
}
