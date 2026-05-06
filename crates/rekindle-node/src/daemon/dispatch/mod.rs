//! Service dispatcher: translates IPC requests into rekindle-transport operations.
//!
//! This module is the bridge between the IPC bus (zero Veilid knowledge) and
//! the rekindle-transport crate (all Veilid operations). Every `IpcRequest`
//! variant is matched in [`dispatch()`] and routed to the appropriate handler
//! in a domain-specific submodule.
//!
//! # Module structure
//!
//! - `mod.rs`       — DaemonContext, dispatch() router, shared helpers
//! - `lifecycle.rs` — Status, Unlock, Lock, Shutdown
//! - `identity.rs`  — IdentityCreate, Show, Export, Rotate, Destroy, Wipe
//! - `community.rs` — CommunityCreate, Join, Leave, List, Info
//! - `channel.rs`   — ChannelList, Create, Delete, Update, Send, History
//! - `social.rs`    — Friend*, Dm*
//! - `governance.rs` — Role*, moderation (Kick/Ban/Unban/Timeout), invites
//! - `keys.rs`      — Mek*, PrekeyReplenish
//! - `presence.rs`  — PresenceSet, GamePresence*, VoiceJoin, VoiceLeave
//! - `admin.rs`     — Agent*, Policy, Subscribe, Unsubscribe, Network*


mod admin;
mod channel;
mod community;
mod governance;
mod identity;
mod keys;
mod lifecycle;
mod presence;
mod social;

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::{
    Session, TransportNode,
    crypto::mek::MekCache,
};

use crate::daemon::DaemonState;
use crate::ipc::protocol::{IpcRequest, IpcResponse};
use crate::ipc::registry::ClearanceRegistry;
use crate::state::keystore::SigningKeyHandle;

/// Active authorization policy loaded from disk.
///
/// Admin policy constraints that cannot be overridden by user config.
/// Fields are additive: they set minimums/maximums, they never disable
/// features that users enabled.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Minimum allowed hop_count for any safety profile.
    pub min_hop_count: Option<u8>,
    /// Whether signature verification can be disabled.
    #[serde(default)]
    pub require_signature_verification: bool,
    /// Maximum allowed gossip TTL.
    pub max_gossip_ttl: Option<u8>,
}

/// Shared daemon context accessible by all dispatch handlers.
///
/// Owns all stateful resources: transport node, session, MEK cache,
/// signing key handle, lifecycle state, agent registry, and policy.
/// The IPC server holds an `Arc<DaemonContext>` and passes it to
/// dispatch on every request.
///
/// ## Lock strategy
///
/// Two lock crate types are used intentionally:
///
/// - `parking_lot::RwLock` — for state accessed synchronously in hot paths
///   (session reads during every IPC request, MEK cache lookups during message
///   sends). parking_lot never yields to the tokio scheduler, so it's safe to
///   hold across non-async code without risking deadlock from task migration.
///
/// - `tokio::sync::RwLock` — for the agent registry, which is mutated during
///   connection handshakes (async context with await points inside the critical
///   section). tokio's RwLock cooperates with the scheduler, preventing a blocked
///   write from starving the runtime.
///
/// Rule: if the critical section contains `await`, use `tokio::sync`. Otherwise,
/// use `parking_lot`.
pub struct DaemonContext {
    /// The transport node (Veilid bridge). None before Veilid is started.
    pub transport: RwLock<Option<Arc<TransportNode>>>,
    /// The user's session (loaded from session.json). None if no identity.
    pub session: Arc<RwLock<Option<Session>>>,
    /// MEK cache shared with transport operations.
    pub mek_cache: Arc<RwLock<MekCache>>,
    /// Ed25519 signing key, held in memory after unlock. None when locked.
    /// Zeroized on drop (via `SigningKeyHandle`'s `ZeroizeOnDrop` derive).
    /// Arc-wrapped to share with DaemonHandler for MEK wrapping during RPC.
    pub signing_key: Arc<RwLock<Option<SigningKeyHandle>>>,
    /// Current daemon lifecycle state.
    pub lifecycle: Arc<crate::daemon::DaemonLifecycle>,
    /// Path to session.json for persistence.
    pub session_path: std::path::PathBuf,
    /// Agent identity and clearance registry (shared with server).
    pub registry: Arc<tokio::sync::RwLock<ClearanceRegistry>>,
    /// Active authorization policy.
    pub policy: RwLock<PolicyConfig>,
    /// Path to the policy config directory (for reload).
    pub config_dir: std::path::PathBuf,
    /// BLAKE3 hash-chained audit logger for all dispatched IPC requests.
    pub audit: parking_lot::Mutex<Option<crate::state::audit::AuditLogger>>,
    /// Consolidated inbound event manager (watch + gossip + poll → dedup → emit).
    /// None before unlock/resume. Created during Resuming transition.
    pub subscriptions: RwLock<Option<rekindle_transport::SubscriptionManager>>,
    /// Consolidated outbound broadcast manager (gossip mesh + rate limiting).
    /// None before unlock/resume. Created during Resuming transition.
    pub broadcast_mgr: RwLock<Option<rekindle_transport::BroadcastManager>>,
    /// Watch channel sender for notifying the IPC server when the subscription
    /// event source becomes available (on unlock) or unavailable (on lock).
    /// The server awaits `.changed()` and starts/stops its delivery task.
    pub event_watch_tx: tokio::sync::watch::Sender<Option<tokio::sync::broadcast::Sender<rekindle_types::subscription_events::SubscriptionEvent>>>,
    /// Pending community join completions. Keyed by governance_key.
    /// Shared between DaemonContext (writer: handle_join registers) and
    /// DaemonHandler (writer: on_gossip completes on JoinAccepted receipt).
    /// Tuple: (oneshot sender carrying slot_index, creation instant for cleanup).
    pub pending_joins: Arc<parking_lot::Mutex<std::collections::HashMap<String, (tokio::sync::oneshot::Sender<u32>, std::time::Instant)>>>,
}

/// Dispatch an IPC request to the appropriate domain handler.
///
/// Returns an `IpcResponse` for every request — no request goes unanswered.
/// Every `IpcRequest` variant is explicitly matched; there is no catch-all arm.
pub async fn dispatch(ctx: &DaemonContext, request: IpcRequest) -> IpcResponse {
    let state = ctx.lifecycle.state();

    // Audit: log every request before dispatch (best-effort, non-blocking).
    audit_request(ctx, &request);

    match request {
        // ── Lifecycle (any state) ────────────────────────────────
        IpcRequest::Status => lifecycle::handle_status(ctx, state),
        IpcRequest::Unlock { passphrase } => lifecycle::handle_unlock(ctx, state, &passphrase).await,
        IpcRequest::Lock => lifecycle::handle_lock(ctx),
        IpcRequest::Shutdown => lifecycle::handle_shutdown(ctx),

        // ── Identity ─────────────────────────────────────────────
        IpcRequest::IdentityCreate { display_name } => identity::handle_create(ctx, state, &display_name).await,
        IpcRequest::IdentityShow => identity::handle_show(ctx, state),
        IpcRequest::IdentityExport => identity::handle_export(ctx, state),
        IpcRequest::IdentityRotate => identity::handle_rotate(ctx, state).await,
        IpcRequest::IdentityDestroy { confirmation } => identity::handle_destroy(ctx, state, &confirmation).await,
        IpcRequest::IdentityWipe { confirmation } => identity::handle_wipe(ctx, state, &confirmation).await,

        // ── Community ────────────────────────────────────────────
        IpcRequest::CommunityCreate { name, description } => community::handle_create(ctx, state, &name, &description).await,
        IpcRequest::CommunityJoin { invite } => community::handle_join(ctx, state, &invite).await,
        IpcRequest::CommunityLeave { governance_key } => community::handle_leave(ctx, state, &governance_key).await,
        IpcRequest::CommunityList => community::handle_list(ctx, state),
        IpcRequest::CommunityInfo { governance_key } => community::handle_info(ctx, state, &governance_key).await,
        IpcRequest::CommunityApprove { governance_key, member_pseudonym } =>
            community::handle_approve(ctx, state, &governance_key, &member_pseudonym).await,
        IpcRequest::CommunityReject { governance_key, member_pseudonym, reason } =>
            community::handle_reject(ctx, state, &governance_key, &member_pseudonym, &reason).await,
        IpcRequest::CommunityPendingMembers { governance_key } =>
            community::handle_pending_members(ctx, state, &governance_key).await,
        IpcRequest::CommunityTransferOwnership { governance_key, new_owner_pseudonym } =>
            community::handle_transfer_ownership(ctx, state, &governance_key, &new_owner_pseudonym).await,

        // ── Channel ──────────────────────────────────────────────
        IpcRequest::ChannelList { community } => channel::handle_list(ctx, state, &community).await,
        IpcRequest::ChannelCreate { community, name, kind, category, topic, slowmode_seconds } =>
            channel::handle_create(ctx, state, &community, &name, &kind, category.as_deref(), topic.as_deref(), slowmode_seconds).await,
        IpcRequest::ChannelDelete { community, channel_id } => channel::handle_delete(ctx, state, &community, &channel_id).await,
        IpcRequest::ChannelUpdate { community, channel_id, name, topic, slowmode_seconds } =>
            channel::handle_update(ctx, state, &community, &channel_id, name.as_deref(), topic.as_deref(), slowmode_seconds).await,
        IpcRequest::ChannelSend { community, channel, body, reply_to } =>
            channel::handle_send(ctx, state, &community, &channel, &body, reply_to).await,
        IpcRequest::ChannelHistory { community, channel, limit } =>
            channel::handle_history(ctx, state, &community, &channel, limit).await,

        // ── Social (friends + DMs) ───────────────────────────────
        IpcRequest::FriendAdd { target, message } => social::handle_friend_add(ctx, state, &target, &message).await,
        IpcRequest::FriendAccept { public_key } => social::handle_friend_accept(ctx, state, &public_key).await,
        IpcRequest::FriendReject { public_key } => social::handle_friend_reject(ctx, state, &public_key).await,
        IpcRequest::FriendRemove { public_key } => social::handle_friend_remove(ctx, state, &public_key).await,
        IpcRequest::FriendList => social::handle_friend_list(ctx, state).await,
        IpcRequest::FriendRequests => social::handle_friend_requests(ctx, state),
        IpcRequest::DmSend { peer_key, body } => social::handle_dm_send(ctx, state, &peer_key, &body).await,
        IpcRequest::DmTyping { peer_key, typing } => social::handle_dm_typing(ctx, state, &peer_key, typing).await,
        IpcRequest::DmInbox { limit } => social::handle_dm_inbox(ctx, state, limit).await,

        // ── Governance (roles, moderation, invites) ──────────────
        IpcRequest::RoleList { community } => governance::handle_role_list(ctx, state, &community).await,
        IpcRequest::RoleCreate { community, name, permissions, color, position } =>
            governance::handle_role_create(ctx, state, &community, &name, permissions, color, position).await,
        IpcRequest::RoleUpdate { community, role_id, name, permissions, color } =>
            governance::handle_role_update(ctx, state, &community, role_id, name.as_deref(), permissions, color).await,
        IpcRequest::RoleDelete { community, role_id } => governance::handle_role_delete(ctx, state, &community, role_id).await,
        IpcRequest::RoleAssign { community, member_pseudonym, role_id } =>
            governance::handle_role_assign(ctx, state, &community, &member_pseudonym, role_id).await,
        IpcRequest::RoleUnassign { community, member_pseudonym, role_id } =>
            governance::handle_role_unassign(ctx, state, &community, &member_pseudonym, role_id).await,
        IpcRequest::Kick { community, target_pseudonym } =>
            governance::handle_kick(ctx, state, &community, &target_pseudonym),
        IpcRequest::Ban { community, target_pseudonym, reason } =>
            governance::handle_ban(ctx, state, &community, &target_pseudonym, reason.as_deref()).await,
        IpcRequest::Unban { community, target_pseudonym } =>
            governance::handle_unban(ctx, state, &community, &target_pseudonym).await,
        IpcRequest::Timeout { community, target_pseudonym, duration_seconds, reason } =>
            governance::handle_timeout(ctx, state, &community, &target_pseudonym, duration_seconds, reason.as_deref()),
        IpcRequest::BanList { community } => governance::handle_ban_list(ctx, state, &community).await,
        IpcRequest::InviteCreate { community, max_uses, expires_seconds } =>
            governance::handle_invite_create(ctx, state, &community, max_uses, expires_seconds).await,
        IpcRequest::InviteList { community } => governance::handle_invite_list(ctx, state, &community).await,
        IpcRequest::InviteRevoke { community, invite_code } =>
            governance::handle_invite_revoke(ctx, state, &community, &invite_code).await,

        // ── Keys ─────────────────────────────────────────────────
        IpcRequest::MekList { community } => keys::handle_mek_list(ctx, state, &community),
        IpcRequest::MekRotate { community, channel } => keys::handle_mek_rotate(ctx, state, &community, &channel).await,
        IpcRequest::MekRequest { community, channel, generation } =>
            keys::handle_mek_request(ctx, state, &community, &channel, generation),
        IpcRequest::PrekeyReplenish => keys::handle_prekey_replenish(ctx, state).await,

        // ── Presence + Voice ─────────────────────────────────────
        IpcRequest::PresenceSet { status, message } => presence::handle_set(ctx, state, &status, message.as_deref()).await,
        IpcRequest::GamePresenceSet { game_name, game_id, elapsed_seconds, server_address } =>
            presence::handle_game_set(ctx, state, &game_name, game_id, elapsed_seconds, server_address.as_deref()).await,
        IpcRequest::GamePresenceClear => presence::handle_game_clear(ctx, state).await,
        IpcRequest::VoiceJoin { community, channel, muted, deafened } =>
            presence::handle_voice_join(ctx, state, &community, &channel, muted, deafened).await,
        IpcRequest::VoiceLeave => presence::handle_voice_leave(ctx, state),

        // ── Admin / Network ──────────────────────────────────────
        // Subscribe/Unsubscribe are handled server-side in the IPC bus router.
        // They never reach daemon dispatch — the server intercepts them to
        // register filters in the EventRouter before routing to daemon.
        // If they arrive here, something bypassed the server layer.
        IpcRequest::Subscribe { .. } => IpcResponse::error(400, "subscribe must be handled by the IPC bus server, not daemon dispatch"),
        IpcRequest::Unsubscribe { .. } => IpcResponse::error(400, "unsubscribe must be handled by the IPC bus server, not daemon dispatch"),
        IpcRequest::NetworkStatus => admin::handle_network_status(ctx, state),
        IpcRequest::NetworkPeers => admin::handle_network_peers(ctx, state),
        IpcRequest::AgentRegister { name, agent_type, capabilities } =>
            admin::handle_agent_register(ctx, &name, agent_type, &capabilities),
        IpcRequest::AgentRevoke { name } => admin::handle_agent_revoke(ctx, &name),
        IpcRequest::PolicyReload => admin::handle_policy_reload(ctx),
    }
}

// ── Shared helpers used across dispatch submodules ───────────────────────

impl DaemonContext {
    /// Get a reference to the transport node, or return a 503 error response.
    pub(crate) fn require_transport(&self) -> Result<Arc<TransportNode>, IpcResponse> {
        self.transport.read().clone().ok_or_else(|| {
            IpcResponse::error_with_remediation(
                503,
                "transport not started — daemon not yet operational",
                "wait for the daemon to reach operational state, then retry",
            )
        })
    }

    /// Briefly hold the session lock, apply a closure, or return 404.
    pub(crate) fn require_session<F, T>(&self, f: F) -> Result<T, IpcResponse>
    where
        F: FnOnce(&Session) -> T,
    {
        let guard = self.session.read();
        guard.as_ref().map(f).ok_or_else(|| {
            IpcResponse::error_with_remediation(
                404,
                "no identity loaded — run: rekindle init",
                "initialize an identity first: rekindle init",
            )
        })
    }

    /// Get the signing key bytes, or return 403 if locked.
    pub(crate) fn require_signing_key(&self) -> Result<[u8; 32], IpcResponse> {
        let guard = self.signing_key.read();
        guard.as_ref().map(|k| *k.as_bytes()).ok_or_else(|| {
            IpcResponse::error_with_remediation(
                403,
                "signing key not available — daemon is locked",
                "unlock the daemon first: rekindle unlock",
            )
        })
    }

    /// Save session to disk, returning an IPC error on failure.
    pub(crate) fn save_session(&self) -> Result<(), IpcResponse> {
        let guard = self.session.read();
        if let Some(ref session) = *guard {
            crate::state::save_session(session, &self.session_path).map_err(|e| {
                IpcResponse::error(500, format!("session persistence failed: {e}"))
            })
        } else {
            Ok(())
        }
    }

    /// Resolve a community by governance key or name from session.
    pub(crate) fn resolve_community(
        &self,
        target: &str,
    ) -> Result<rekindle_transport::CommunityMembership, IpcResponse> {
        let guard = self.session.read();
        let session = guard.as_ref().ok_or_else(|| {
            IpcResponse::error(404, "no identity loaded")
        })?;
        // Exact governance key match
        if let Some(m) = session.community(target) {
            return Ok(m.clone());
        }
        // Case-insensitive name match
        if let Some(m) = session.community_by_name(target) {
            return Ok(m.clone());
        }
        Err(IpcResponse::error(404, format!("community '{target}' not found")))
    }

}

/// Produce a standard error response for state violations.
pub(crate) fn state_error(state: DaemonState, required: &str) -> IpcResponse {
    IpcResponse::error_with_remediation(
        409,
        format!("cannot perform {required} operation in state '{}'", state.as_str()),
        if state == DaemonState::Locked {
            "unlock the daemon first: rekindle unlock"
        } else {
            "wait for the daemon to reach operational state"
        },
    )
}

/// Log an IPC request to the BLAKE3 hash-chained audit log.
///
/// Best-effort: audit failures are logged but never block the request.
/// The audit entry includes the request type and security-relevant context
/// but never the payload body (no message content in the audit trail).
fn audit_request(ctx: &DaemonContext, request: &IpcRequest) {
    let event_type = format!("{request:?}");
    // Truncate to just the variant name for the audit log (no field data)
    let event_name = event_type.split_once(' ')
        .or_else(|| event_type.split_once('{'))
        .map_or(event_type.as_str(), |(name, _)| name)
        .trim();

    let mut guard = ctx.audit.lock();
    if let Some(ref mut logger) = *guard {
        if let Err(e) = logger.append(
            event_name.as_bytes(),
            None, // sender name filled by server layer
            crate::ipc::message::SecurityLevel::Open,
            event_name.to_string(),
            None,
        ) {
            tracing::debug!(error = %e, event = event_name, "audit log write failed (non-fatal)");
        }
    }
}

// ── Daemon bus subscriber ───────────────────────────────────────────────

impl DaemonContext {
    /// Run the daemon as a bus subscriber.
    ///
    /// Connects to the daemon's own IPC socket as a privileged internal
    /// agent, receives `BusPayload::Request` messages routed by the server,
    /// dispatches each to the appropriate handler, and sends correlated
    /// `BusPayload::Response` messages back through the bus.
    ///
    /// This method runs until the bus connection is closed (daemon shutdown).
    pub async fn run_subscriber(
        self: &std::sync::Arc<Self>,
        mut client: crate::ipc::client::BusClient,
    ) {
        tracing::info!("daemon bus subscriber started");

        loop {
            let msg = match client.recv_bus_message().await {
                Some(Ok(msg)) => msg,
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "daemon subscriber: decode failed, skipping");
                    continue;
                }
                None => {
                    tracing::info!("daemon subscriber: bus connection closed");
                    break;
                }
            };

            let request = match msg.payload {
                crate::ipc::protocol::BusPayload::Request(req) => req,
                other => {
                    tracing::debug!(payload = ?std::mem::discriminant(&other), "daemon subscriber: non-request payload, ignoring");
                    continue;
                }
            };

            let correlation_id = msg.msg_id;
            let level = msg.security_level;
            let response = dispatch(self, request).await;

            // Serialize IpcResponse to JSON bytes for the bus wire format.
            // IpcResponse contains serde_json::Value which postcard cannot handle.
            let response_bytes = match serde_json::to_vec(&response) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = %e, "daemon subscriber: failed to serialize response");
                    continue;
                }
            };

            if let Err(e) = client.respond(response_bytes, correlation_id, level).await {
                tracing::error!(error = %e, "daemon subscriber: failed to send response");
            }
        }

        tracing::info!("daemon bus subscriber stopped");
    }
}
