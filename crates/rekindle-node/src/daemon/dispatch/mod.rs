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
//! - `friends.rs`   — Friend*, Dm*, SessionReset* (over `operations::friend`/`dm`)
//! - `governance.rs` — Role*, moderation (Kick/Ban/Unban/Timeout), invites
//! - `keys.rs`      — Mek*
//! - `own.rs`       — our presence, profile, game, audit log, links
//! - `voice.rs`     — VoiceJoin, VoiceLeave
//! - `admin.rs`     — Agent*, Policy, Subscribe, Unsubscribe, Network*

mod admin;
mod audit;
mod automod;
pub(crate) mod channel;
mod channel_admin;
mod channel_message;
mod channel_message_ops;
pub(crate) mod community;
mod community_profile;
mod context;
mod events;
mod friends;
mod game_servers;
pub(crate) mod governance;
mod identity;
mod in_flight;
mod keys;
mod lifecycle;
mod member_block;
mod onboarding;
mod own;
mod pins;
mod rich_messaging;
mod router;
pub(crate) mod shell;
mod status;
mod unread;
mod voice;

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::{crypto::mek::MekCache, Session, TransportNode};

use crate::state::signing_key::SigningKeyHandle;
use rekindle_ipc::registry::ClearanceRegistry;

pub(crate) use context::{runtime_refused, state_error, transition};
pub(crate) use lifecycle::{daemon_stages, daemon_teardown};
pub use router::dispatch;

use crate::host::policy::PolicyConfig;

/// Who sent the request being dispatched (`host::api::Caller`).
pub use rekindle_ipc::Caller;

/// Build a governance adapter over this request's context.
///
/// One definition rather than one per dispatch module: the body is
/// three tokens, which is exactly the size at which copies stop looking
/// worth converging and start drifting.
pub(crate) fn adapter(ctx: &DaemonContext) -> crate::host::adapters::governance::HostGovernance {
    crate::host::adapters::governance::HostGovernance::new(
        ctx.this.upgrade().expect("a context in use is alive"),
    )
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
    /// Daemon or shell (`host::runtime::HostMode`).
    pub mode: crate::host::HostMode,
    /// The transport node (Veilid bridge). None before Veilid is started.
    pub transport: RwLock<Option<Arc<TransportNode>>>,
    /// The unlocked identity's session, projected from its stores at
    /// unlock (`host::session`). None while locked.
    pub session: Arc<RwLock<Option<Session>>>,
    /// The data root: the identity registry and every identity's files.
    pub root: rekindle_db::paths::DataRoot,
    /// The unlocked identity's database (plan D1), shared with the
    /// handler and the inbox scan. Empty while locked.
    pub db: Arc<rekindle_db::DbHandle>,
    /// The unlocked identity's vault. Empty while locked.
    pub vault: rekindle_vault::VaultHandle,
    /// The one wrong-passphrase counter per identity (U1.9): unlock,
    /// import, rotate and export count on it.
    pub identity_attempts: std::sync::Arc<rekindle_idempotency::attempts::AttemptLimiter<String>>,
    /// MEK cache shared with transport operations.
    pub mek_cache: Arc<RwLock<MekCache>>,
    /// Ed25519 signing key, held in memory after unlock. None when locked.
    /// Zeroized on drop (via `SigningKeyHandle`'s `ZeroizeOnDrop` derive).
    /// Arc-wrapped to share with DaemonHandler for MEK wrapping during RPC.
    pub signing_key: Arc<RwLock<Option<SigningKeyHandle>>>,
    /// Current daemon lifecycle state.
    pub lifecycle: Arc<crate::daemon::DaemonLifecycle>,
    /// Agent identity and clearance registry (shared with server).
    pub registry: Arc<tokio::sync::RwLock<ClearanceRegistry>>,
    /// The registry names of the unlocked identity's clients
    /// (`daemon::clients`), removed again at lock.
    pub unlocked_clients: parking_lot::Mutex<Vec<String>>,
    /// Active authorization policy.
    pub policy: RwLock<PolicyConfig>,
    /// Path to the policy config directory (for reload).
    pub config_dir: std::path::PathBuf,
    /// The unlocked identity's tamper-evident audit chain
    /// (`host::audit`); `None` while locked, when nothing is written.
    pub audit_chain: std::sync::Arc<crate::host::audit::ChainSlot>,
    /// Consolidated inbound event manager (watch + gossip + poll → dedup → emit).
    /// None before unlock/resume. Created during Resuming transition. The
    /// transport's inbound handler holds the same slot, so the gossip and
    /// value changes it receives reach the manager installed here.
    pub subscriptions: std::sync::Arc<RwLock<Option<rekindle_transport::SubscriptionManager>>>,
    /// Consolidated outbound broadcast manager (gossip mesh + rate limiting).
    /// None before unlock/resume. Created during Resuming transition.
    pub broadcast_mgr: RwLock<Option<rekindle_transport::BroadcastManager>>,
    /// The host's one intent gate (plan D3.3), counting wrong passphrase
    /// proofs on `identity_attempts`.
    pub intents: std::sync::Arc<crate::host::gate::HostGate>,
    /// The undo offers of actions that ran (plan E1.1).
    pub undo: crate::host::undo::UndoCache,
    /// The host's one event journal (plan D3.2): every event a frontend
    /// observes is published here, and every subscriber reads from it.
    pub journal: std::sync::Arc<rekindle_events::SessionJournal>,
    /// The host's wall clock (plan C9.2): every timestamp the host and its
    /// engines write or compare. `SystemClock` in production.
    pub clock: Arc<dyn rekindle_utils::Clock>,
    /// The host's generator for the ids, keys and jitter it and its engines
    /// mint (plan C9.2). `OsRandom` in production; AEAD nonces never come
    /// from it.
    pub rng: Arc<dyn rekindle_utils::RandomSource>,
    /// The user's input idle time (plan E2.B): `rekindle_idle::OsIdle` in
    /// production.
    pub idle: Arc<dyn rekindle_utils::IdleSource>,
    /// The transport's subscription event source while unlocked: the
    /// forwarder (`daemon::event_forward`) journals what it carries.
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
    /// Each community's compiled automod rules, which the channel pipeline
    /// rebuilds when the rules' fingerprint changes (`rekindle_channel::automod`).
    pub automod_rules: parking_lot::RwLock<
        std::collections::HashMap<String, Arc<rekindle_channel::AutoModCompiledCache>>,
    >,
    /// The Host's one community store (plan E3.1, `host::community`): the
    /// cached CRDT `GovernanceState`, leases, roster and roles, over the
    /// map the GUI's `AppState.communities` shares in Shell mode.
    pub communities: Arc<crate::host::community::CommunityStore>,
    /// The unlocked identity's chunk caches (plan E3.1 E3c,
    /// `host::adapters::files`).
    pub files: Arc<rekindle_files::FileCacheStore>,
    /// Read markers advanced but not yet written (plan E3.1, D25: ≤ 30 s
    /// write-behind; `host::read_markers`).
    pub read_markers: crate::host::read_markers::PendingMarkers,
    /// The received posts' slowmode windows, which the D26 gate keeps
    /// (plan E3.1 F3, `host::adapters::channel::ingest`).
    pub channel_rate_limits: parking_lot::Mutex<rekindle_channel::ingest::RateLimits>,
    /// Departure-triggered MEK rotations, queued for the worker that
    /// owns the daemon's `Arc<DaemonContext>`.
    ///
    /// A channel rather than a direct call because rotation waits out
    /// the cascade (tens of seconds) and must not hold an IPC handler
    /// open, and because a detached task needs `'static` state that a
    /// `&DaemonContext` handler cannot give it. See
    /// `daemon::mek_rotation`.
    /// Outbound gossip (PATH 2), queued for the worker that owns the
    /// daemon's `Arc<DaemonContext>`.
    ///
    /// Every `Deps::send_to_mesh` on this track goes through here, so
    /// there is one encoder and one wire format rather than three
    /// partial translations. See `daemon::gossip`.
    pub gossip_tx: crate::daemon::gossip::GossipSender,
    pub mek_rotation_tx: crate::daemon::mek_rotation::MekRotationSender,
    /// The unlock's scope (plan C4): every task that needs the unlocked
    /// identity runs in it, and `host::unlock::teardown` shuts it down
    /// first. `None` while locked. A panic in one of its tasks exits the
    /// daemon (`ExitReason::HandlerPanic`) and ends the GUI's session.
    pub unlock_scope: RwLock<Option<Arc<rekindle_lifecycle::SessionScope>>>,
    /// Per-community children of the unlock scope (the presence poll),
    /// keyed by governance key. The polls hold an `Arc<DaemonContext>` and
    /// write presence rows, so leaving a community shuts its scope down
    /// and locking shuts them all down with the unlock scope.
    pub community_scopes: parking_lot::Mutex<
        std::collections::HashMap<String, Arc<rekindle_lifecycle::SessionScope>>,
    >,
    /// Asks the presence supervisor to start polls. Sent on unlock,
    /// once the signing key and broadcast manager exist.
    pub presence_start_tx: crate::daemon::presence_adapter::supervisor::PresenceStartSender,
    /// Our own presence, one owner on both hosts (plan E1.1): status,
    /// message, game and idle state, and the unlock's STATUS publisher.
    pub(crate) own_presence: crate::host::own_presence::OwnPresence,
    /// The dispatch lane (`IpcRequest::lane`): `Write` requests hold it
    /// shared, `Exclusive` ones alone. A lock, unlock or identity change
    /// therefore never overlaps a write that may hold the signing key.
    pub write_lane: tokio::sync::RwLock<()>,
    /// The bus subscriber's liveness, which gates the systemd watchdog.
    pub subscriber_heartbeat: crate::daemon::heartbeat::Heartbeat,
    /// The daemon-wide shutdown signal and the reason it was raised.
    /// Shared with the unlock scope's panic hook.
    pub shutdown: Arc<crate::daemon::shutdown::Shutdown>,
    /// The GUI's side of an unlock (`host::shell_hooks`); `None` in
    /// `rekindled`.
    pub shell: Option<Arc<dyn crate::host::shell_hooks::ShellHooks>>,
    /// A panicked unlock task's name, for the shell's teardown worker
    /// (`host::unlock::run_panic_teardowns`).
    pub unlock_panics: tokio::sync::mpsc::UnboundedSender<String>,
    /// The receive gate's friend authority over the unlocked identity's
    /// database (plan E2.1); `None` while locked.
    pub friend_store: RwLock<Option<Arc<rekindle_db::SqliteFriendStore>>>,
    /// The 1:1 replay guard and session-reset state of the unlocked
    /// identity (plan E2.1), cleared at lock.
    pub friend_stores: rekindle_transport::operations::friend::FriendStores,
    /// The unlocked identity's outbox (plan E2.5, D22).
    pub(crate) outbox: crate::host::adapters::outbox::OutboxSlot,
    /// Each watched friend's profile lease (it carries the watch), and the
    /// friends whose watch failed, polled instead (plan E2.1).
    pub(crate) friend_watches: crate::host::adapters::friends::FriendWatches,
    /// Our profile record's session lease, so a rotation releases it.
    pub(crate) own_profile_lease: Arc<parking_lot::Mutex<Option<rekindle_records::lease::LeaseId>>>,
    /// Our prekey record's session lease (plan E2.4), set by the own-records
    /// stage; the maintenance and the refills write through it.
    pub(crate) own_prekey_lease: Arc<parking_lot::Mutex<Option<rekindle_records::lease::LeaseId>>>,
    /// Held by every writer of our prekey record (the own-records stage
    /// from its reconcile through its later slots, each maintenance pass,
    /// and a rotation from its set until the unlock ends): a reconcile
    /// compares the record with our rows, so one running beside a rotation
    /// would put the old set back over the new one (E2.B finding F9).
    pub(crate) own_prekey_writes: Arc<tokio::sync::Mutex<()>>,
    /// This context, for work that outlives a call (`HostFriends::owned`).
    pub(crate) this: std::sync::Weak<DaemonContext>,
}

impl DaemonContext {
    /// The unlock scope, or a closed one while locked, so work spawned
    /// with no unlocked identity to own it is dropped.
    pub fn unlock_scope_or_closed(&self) -> Arc<rekindle_lifecycle::SessionScope> {
        self.unlock_scope
            .read()
            .clone()
            .unwrap_or_else(|| rekindle_lifecycle::SessionScope::closed("unlock"))
    }

    /// The scope of one community's tasks, a child of the unlock scope
    /// created on first use; a closed scope while locked.
    pub fn community_scope(&self, community_id: &str) -> Arc<rekindle_lifecycle::SessionScope> {
        let Some(unlock) = self.unlock_scope.read().clone() else {
            return rekindle_lifecycle::SessionScope::closed("community");
        };
        Arc::clone(
            self.community_scopes
                .lock()
                .entry(community_id.to_string())
                .or_insert_with(|| unlock.child("community")),
        )
    }

    /// Our pseudonym in one community, or `""` when we are not a member.
    ///
    /// Every adapter needs this and three of them had written the same
    /// session walk. It reads `DaemonContext`, so it belongs on
    /// `DaemonContext` — `services-pattern.md` puts the state access on
    /// the host and leaves the adapter as the trait's shape.
    ///
    /// The empty string is the "not a member" signal because the `Deps`
    /// traits return `String`, not `Option<String>`; callers compare
    /// against it.
    pub fn my_pseudonym(&self, community_id: &str) -> String {
        self.session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.pseudonym_key.clone())
            .unwrap_or_default()
    }

    /// The unlocked Ed25519 secret, or `None` while locked.
    ///
    /// Copies the bytes out rather than lending the handle: the handle
    /// is `ZeroizeOnDrop` and must not escape the lock guard.
    pub fn identity_secret(&self) -> Option<[u8; 32]> {
        self.signing_key.read().as_ref().map(|k| *k.as_bytes())
    }

    /// Our display name, or `""` before an identity is loaded.
    pub fn identity_display_name(&self) -> String {
        self.session
            .read()
            .as_ref()
            .map(|s| s.identity.display_name.clone())
            .unwrap_or_default()
    }

    /// Journal one `SubscriptionEvent` for every subscriber (plan D3.2).
    pub fn publish_event(&self, event: rekindle_types::subscription_events::SubscriptionEvent) {
        self.journal.publish(event);
    }
}
