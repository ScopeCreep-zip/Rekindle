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
mod context;
mod governance;
mod identity;
mod keys;
mod lifecycle;
mod presence;
mod router;
mod social;

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::{crypto::mek::MekCache, Session, TransportNode};

use crate::ipc::registry::ClearanceRegistry;
use crate::state::keystore::SigningKeyHandle;

pub(crate) use context::state_error;
pub use router::dispatch;

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
    pub event_watch_tx: tokio::sync::watch::Sender<
        Option<
            tokio::sync::broadcast::Sender<rekindle_types::subscription_events::SubscriptionEvent>,
        >,
    >,
    /// Pending community join completions. Keyed by governance_key.
    /// Shared between DaemonContext (writer: handle_join registers) and
    /// DaemonHandler (writer: on_gossip completes on JoinAccepted receipt).
    /// Tuple: (oneshot sender carrying slot_index, creation instant for cleanup).
    pub pending_joins: Arc<
        parking_lot::Mutex<
            std::collections::HashMap<
                String,
                (tokio::sync::oneshot::Sender<u32>, std::time::Instant),
            >,
        >,
    >,
    /// Per-community runtime state backing the governance adapter —
    /// cached CRDT `GovernanceState` and open-record tracking. Not
    /// persisted; see `daemon::community_runtime`.
    pub community_runtime: Arc<crate::daemon::community_runtime::CommunityRuntimeMap>,
}
