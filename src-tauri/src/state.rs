use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
pub use rekindle_gossip::dedup::DedupCache;
pub use rekindle_gossip::mesh::fanout_degree as gossip_degree;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Central application state shared across all Tauri commands and services.
pub struct AppState {
    /// Channel for routing incoming voice packets from the dispatch loop to the receive loop.
    pub voice_packet_tx: Arc<RwLock<Option<mpsc::Sender<rekindle_voice::transport::VoicePacket>>>>,
    /// Identity (loaded after Stronghold unlock).
    pub identity: Arc<RwLock<Option<IdentityState>>>,
    /// Friends list with presence info.
    pub friends: Arc<RwLock<HashMap<String, FriendState>>>,
    /// Joined communities.
    pub communities: Arc<RwLock<HashMap<String, CommunityState>>>,
    /// Veilid node handle (set after login/attach).
    pub node: Arc<RwLock<Option<NodeHandle>>>,
    /// DHT record manager for reading/writing distributed state.
    pub dht_manager: Arc<RwLock<Option<DHTManagerHandle>>>,
    /// Routing manager for private route lifecycle.
    pub routing_manager: Arc<RwLock<Option<RoutingManagerHandle>>>,
    /// Signal session manager (set after identity unlock).
    pub signal_manager: Arc<Mutex<Option<SignalManagerHandle>>>,
    /// Game detector state.
    pub game_detector: Arc<Mutex<Option<GameDetectorHandle>>>,
    /// Voice engine state.
    pub voice_engine: Arc<Mutex<Option<VoiceEngineHandle>>>,
    /// Channel for sending shutdown signals to background services.
    pub shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    /// Channel for sending shutdown signal to the sync service.
    pub sync_shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    /// Sender half of the watch channel that tracks Veilid public internet readiness.
    /// Set to `true` when the network is ready for DHT operations, `false` otherwise.
    pub network_ready_tx: Arc<tokio::sync::watch::Sender<bool>>,
    /// Receiver half — clone and `.changed().await` to wait for readiness.
    pub network_ready_rx: tokio::sync::watch::Receiver<bool>,
    /// Ed25519 secret key bytes for signing `MessageEnvelope`s.
    /// Stored after identity unlock so `message_service` can sign outgoing messages.
    pub identity_secret: Mutex<Option<[u8; 32]>>,
    /// Handles for spawned background tasks (DHT publish, retry init, etc.)
    /// that don't have their own shutdown channels. Aborted on logout to prevent
    /// stale tasks from interfering with re-login.
    pub background_handles: Mutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
    /// MEK cache: `community_id` -> current `MediaEncryptionKey` (from JoinAccepted or DHT vault).
    /// Legacy community-level MEK — used during the transition to per-channel MEK.
    pub mek_cache: Mutex<HashMap<String, MediaEncryptionKey>>,
    /// Per-channel MEK cache: `(community_id, channel_id)` -> `MediaEncryptionKey`.
    /// New per-channel MEK distribution: each channel has its own encryption key.
    pub channel_mek_cache: Mutex<HashMap<(String, String), MediaEncryptionKey>>,
    /// Friends whose DHT `watch_dht_values` returned false (watch not established).
    /// Per Veilid GitLab #377, apps must poll as fallback when watching fails.
    /// The sync service uses `force_refresh=true` for these friends.
    pub unwatched_friends: RwLock<HashSet<String>>,
    /// `JoinHandle` for the Veilid dispatch loop (started at app launch, awaited on exit).
    pub dispatch_loop_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Shutdown sender for the route refresh loop (stored here so it outlives `background_handles`).
    pub route_refresh_shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
    /// Shutdown sender for the idle/auto-away service.
    pub idle_shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
    /// Shutdown sender for the presence heartbeat loop.
    pub heartbeat_shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
    /// The status the user had before auto-away kicked in.
    /// When activity resumes, we restore to this status.
    pub pre_away_status: RwLock<Option<UserStatus>>,
    /// Deep link action received before the user was authenticated.
    /// Replayed after successful login so the user isn't silently dropped.
    pub pending_deep_link: Mutex<Option<crate::deep_links::DeepLinkAction>>,
    /// Per-community circuit breaker for remote Veilid RPCs.
    /// Prevents flooding dead routes with parallel 8s timeouts.
    /// In-memory only, resets on app restart.
    pub community_circuit_breakers: RwLock<HashMap<String, CircuitBreakerState>>,
    /// Tauri app handle — set during `.setup()`, used by background services
    /// (community gossip, heartbeat, etc.) to emit events and access managed state.
    pub app_handle: RwLock<Option<tauri::AppHandle>>,
    /// Global dedup cache for gossip mesh message deduplication.
    /// Prevents processing/forwarding the same message twice.
    pub dedup_cache: Mutex<DedupCache>,
    /// Sender for queued SMPL channel-message writes.
    pub channel_write_retry_tx: Arc<RwLock<Option<rekindle_records::retry::WriteQueueHandle>>>,
    /// Per-community compiled AutoMod cache.
    pub automod_cache: Arc<RwLock<HashMap<String, AutoModCompiledCache>>>,
    /// Wake-up signal for the event reminder scheduler.
    pub event_reminder_wake_tx: Arc<RwLock<Option<tokio::sync::watch::Sender<u64>>>>,
    /// Per-community Lost Cargo chunk caches. `community_id` → `ChunkCache`.
    /// Initialized lazily on first use of a community's file system. Lives
    /// on the filesystem under `<app_data>/file_cache/<community_id>/`.
    pub file_caches: RwLock<HashMap<String, rekindle_files::ChunkCache>>,
    /// Pinned-attachment IDs per community. Mirrors the merged
    /// `governance_state.pinned_attachments` set; consulted by the cache
    /// during eviction sweeps.
    pub pinned_attachments: RwLock<HashMap<String, rekindle_files::PinnedSet>>,
    /// Root directory for all per-community Lost Cargo caches. Resolved
    /// once at app setup from `app_handle.path().app_data_dir()`.
    pub file_cache_root: RwLock<Option<std::path::PathBuf>>,
    /// DM MEK cache (architecture §27.1). Keyed by SMPL record key →
    /// `DmMekChain` holding the genesis MEK plus every materialized
    /// generation forward. Forward architecture for §5.2 line 1100 +
    /// §5.3 line 1186: every message envelope carries its `mek_generation`,
    /// and receivers must hold historical MEKs to decrypt them.
    pub dm_mek_cache: Mutex<HashMap<String, rekindle_dm::DmMekChain>>,

    /// Strand Relay probe cooldown (architecture §13.5). Keyed by the
    /// target pseudonym hex → unix-seconds of the last `StatusRequest`
    /// fan-out. Prevents amplification when send-failure retries
    /// repeatedly trigger probes (BEP-11-style 60s window).
    pub relay_probe_cooldown: Mutex<HashMap<String, u64>>,

    /// Mutual Aid §14.5 dirty set for per-peer reliability counters.
    /// `record_peer_reliability` updates the in-memory map on every
    /// gossip event and inserts here; a 30s flush task drains this set
    /// into SQLite. Avoids the hot-path-write amplification of
    /// per-event upserts (D-fanout × send/recv ≈ 1000 writes/min in
    /// busy communities).
    pub relay_reliability_dirty: Mutex<HashSet<(String, String)>>,

    /// Push relay (architecture §17.3 Tier 3) wake-notify debounce.
    /// Each `WakeNotify` triggers a 30+ second DHT sync sweep; we cap
    /// to one sweep per debounce window so back-to-back wakes don't
    /// saturate the network or drain mobile battery.
    pub last_wake_notify_secs: Mutex<u64>,

    /// Architecture §10.6 video / screen-share reassembly. Per-community
    /// pending-fragment buffers; cleared on logout via the cleanup path.
    pub video_reassembly:
        crate::services::community::video::VideoReassemblyState,

    /// Architecture §17.2 line 2402 — per-channel notification burst
    /// throttle. Bounds OS popups to 5 per 10 s per channel and folds
    /// the rest into a single summary notification.
    pub notification_throttle: crate::services::community::notifications::NotificationThrottle,

    /// Plan §Failure 5 — direct call state, keyed by `call_id`. Holds
    /// the local X25519 secret + peer public until both sides have
    /// exchanged keys, then derives the symmetric `call_key` consumed
    /// by the voice transport. Entries drop on Active→hangup or on
    /// Outgoing→Missed timeout (which writes a `missed_calls` row).
    pub active_calls: Arc<Mutex<HashMap<String, rekindle_calls::CallState>>>,
}

impl Default for AppState {
    fn default() -> Self {
        let (network_ready_tx, network_ready_rx) = tokio::sync::watch::channel(false);
        Self {
            voice_packet_tx: Arc::new(RwLock::new(None)),
            identity: Arc::new(RwLock::new(None)),
            friends: Arc::new(RwLock::new(HashMap::new())),
            communities: Arc::new(RwLock::new(HashMap::new())),
            node: Arc::new(RwLock::new(None)),
            dht_manager: Arc::new(RwLock::new(None)),
            routing_manager: Arc::new(RwLock::new(None)),
            signal_manager: Arc::new(Mutex::new(None)),
            game_detector: Arc::new(Mutex::new(None)),
            voice_engine: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            sync_shutdown_tx: Arc::new(RwLock::new(None)),
            network_ready_tx: Arc::new(network_ready_tx),
            network_ready_rx,
            identity_secret: Mutex::new(None),
            background_handles: Mutex::new(Vec::new()),
            mek_cache: Mutex::new(HashMap::new()),
            channel_mek_cache: Mutex::new(HashMap::new()),
            unwatched_friends: RwLock::new(HashSet::new()),
            dispatch_loop_handle: RwLock::new(None),
            route_refresh_shutdown_tx: RwLock::new(None),
            idle_shutdown_tx: RwLock::new(None),
            heartbeat_shutdown_tx: RwLock::new(None),
            pre_away_status: RwLock::new(None),
            pending_deep_link: Mutex::new(None),
            community_circuit_breakers: RwLock::new(HashMap::new()),
            app_handle: RwLock::new(None),
            dedup_cache: Mutex::new(DedupCache::new(1024)),
            channel_write_retry_tx: Arc::new(RwLock::new(None)),
            automod_cache: Arc::new(RwLock::new(HashMap::new())),
            event_reminder_wake_tx: Arc::new(RwLock::new(None)),
            file_caches: RwLock::new(HashMap::new()),
            pinned_attachments: RwLock::new(HashMap::new()),
            file_cache_root: RwLock::new(None),
            dm_mek_cache: Mutex::new(HashMap::new()),
            relay_probe_cooldown: Mutex::new(HashMap::new()),
            relay_reliability_dirty: Mutex::new(HashSet::new()),
            last_wake_notify_secs: Mutex::new(0),
            video_reassembly:
                crate::services::community::video::VideoReassemblyState::new(),
            notification_throttle:
                crate::services::community::notifications::NotificationThrottle::new(),
            active_calls: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutoModCompiledCache {
    pub fingerprint: Vec<([u8; 16], u64)>,
    pub rules: Vec<CompiledAutoModRule>,
}

#[derive(Debug, Clone)]
pub struct CompiledAutoModRule {
    pub rule_id: [u8; 16],
    pub name: String,
    pub keywords_lower: Vec<String>,
    pub regexes: Vec<Regex>,
    pub action: String,
}

/// Per-community circuit breaker for remote Veilid RPCs.
///
/// Prevents flooding dead routes with parallel 8s timeouts. Once a community
/// trips (>= 3 consecutive failures), further RPCs are rejected instantly for
/// a 30s cooldown period. Resets on success. In-memory only, resets on restart.
pub struct CircuitBreakerState {
    pub tripped_at: std::time::Instant,
    pub failure_count: u32,
}

// ── Gossip overlay types ──

/// Gossip overlay state for a community.
///
/// Each member maintains a random peer set of D online members and forwards
/// received messages to them. Adaptive degree:
/// - ≤20 members: D = N-1 (direct mesh)
/// - 21-60: D = 6, 61+: D = 8
/// An online community member with their route blob and last-seen timestamp.
#[derive(Debug, Clone)]
pub struct OnlineMember {
    /// Veilid private route blob for reaching this member.
    pub route_blob: Vec<u8>,
    /// Last advertised member status from the registry or gossip mesh.
    pub status: String,
    /// Timestamp (seconds since epoch) of last valid gossip message or presence update.
    /// Used for TTL-based eviction of stale members.
    pub last_seen: u64,
}

#[derive(Debug, Clone)]
pub struct GossipOverlay {
    /// Current gossip peers: pseudonym_key → member info.
    /// These are the D peers we send/forward every message to.
    pub peers: HashMap<String, OnlineMember>,
    /// All online members: pseudonym_key → member info.
    /// Superset of `peers`. Updated on each presence poll.
    pub online_members: HashMap<String, OnlineMember>,
    /// Lamport counter for outgoing messages.
    /// Incremented for each message we originate (not forwards).
    pub lamport_counter: u64,
    /// True until the first successful sync after coming online.
    /// Used to trigger a `SyncRequest` to online peers for catch-up.
    pub needs_initial_sync: bool,
    /// A1/P4.1 — broadcasts queued because `peers` was empty at send time.
    /// `send_to_mesh_raw` enqueues here instead of dropping; the next
    /// presence poll that lands online peers drains the queue and re-sends.
    /// Bounded at 100 (oldest dropped) so an offline burst doesn't OOM.
    /// Without this, the first member of a fresh community broadcast all
    /// their join announcements / MEK requests / governance updates to a
    /// zero-peer mesh — silently lost.
    pub pending_mesh_broadcasts: std::collections::VecDeque<
        rekindle_protocol::dht::community::envelope::SignedEnvelope,
    >,
}

impl Default for GossipOverlay {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            online_members: HashMap::new(),
            lamport_counter: 0,
            needs_initial_sync: true,
            pending_mesh_broadcasts: std::collections::VecDeque::with_capacity(16),
        }
    }
}

/// Shared reference to `AppState`, used by both Tauri commands and background services.
pub type SharedState = Arc<AppState>;

/// Handle to the live Veilid node.
pub struct NodeHandle {
    /// Raw Veilid `AttachmentState` string (e.g. "detached", "attaching", "`attached_good`").
    pub attachment_state: String,
    /// Whether the node is attached to the network.
    pub is_attached: bool,
    /// Whether the public internet is ready for DHT operations.
    pub public_internet_ready: bool,
    /// Veilid API handle (needed for shutdown, route import, etc.).
    pub api: veilid_core::VeilidAPI,
    /// Veilid routing context (needed for `app_message`, DHT ops).
    pub routing_context: veilid_core::RoutingContext,
    /// Our private route blob for receiving messages.
    pub route_blob: Option<Vec<u8>>,
    /// Our DHT profile record key.
    pub profile_dht_key: Option<String>,
    /// Owner keypair for our profile DHT record (needed to re-open with write access).
    pub profile_owner_keypair: Option<veilid_core::KeyPair>,
    /// Our DHT friend list record key.
    pub friend_list_dht_key: Option<String>,
    /// Owner keypair for our friend list DHT record (needed to re-open with write access).
    pub friend_list_owner_keypair: Option<veilid_core::KeyPair>,
    /// Our account DHT record key (Phase 3).
    pub account_dht_key: Option<String>,
    /// Our mailbox DHT record key (deterministic, permanent).
    pub mailbox_dht_key: Option<String>,
}

impl NodeHandle {
    /// Store profile DHT key and owner keypair after publishing.
    pub fn set_profile_dht(&mut self, key: String, keypair: Option<veilid_core::KeyPair>) {
        self.profile_dht_key = Some(key);
        if keypair.is_some() {
            self.profile_owner_keypair = keypair;
        }
    }

    /// Store friend list DHT key and owner keypair after publishing.
    pub fn set_friend_list_dht(&mut self, key: String, keypair: Option<veilid_core::KeyPair>) {
        self.friend_list_dht_key = Some(key);
        if keypair.is_some() {
            self.friend_list_owner_keypair = keypair;
        }
    }

    /// Store account DHT key after publishing.
    pub fn set_account_dht(&mut self, key: String) {
        self.account_dht_key = Some(key);
    }

    /// Store mailbox DHT key after publishing.
    pub fn set_mailbox_dht(&mut self, key: String) {
        self.mailbox_dht_key = Some(key);
    }
}

/// Handle to the Signal session manager.
pub struct SignalManagerHandle {
    /// The crypto session manager.
    pub manager: rekindle_crypto::SignalSessionManager,
}

/// Handle to the game detector.
pub struct GameDetectorHandle {
    /// Shutdown sender for the game detection loop.
    pub shutdown_tx: mpsc::Sender<()>,
    /// Current detected game info.
    pub current_game: Option<GameInfoState>,
}

/// Handle to the voice engine.
pub struct VoiceEngineHandle {
    pub engine: rekindle_voice::VoiceEngine,
    /// Shared voice transport — used by send loop, MCU loop, and VoiceJoin handler.
    pub transport: std::sync::Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
    /// Shutdown sender for the voice send loop task.
    pub send_loop_shutdown: Option<mpsc::Sender<()>>,
    /// Join handle for the voice send loop task.
    pub send_loop_handle: Option<tokio::task::JoinHandle<()>>,
    /// Shutdown sender for the voice receive loop task.
    pub recv_loop_shutdown: Option<mpsc::Sender<()>>,
    /// Join handle for the voice receive loop task.
    pub recv_loop_handle: Option<tokio::task::JoinHandle<()>>,
    /// Shutdown sender for the device monitor loop task.
    pub device_monitor_shutdown: Option<mpsc::Sender<()>>,
    /// Join handle for the device monitor loop task.
    pub device_monitor_handle: Option<tokio::task::JoinHandle<()>>,
    /// Which channel/call we're currently in (prevents double-joining).
    pub channel_id: String,
    /// Community ID if this is a community voice channel (None for DM voice).
    pub community_id: Option<String>,
    /// Shutdown sender for the MCU mix loop task (group voice host only).
    pub mcu_loop_shutdown: Option<mpsc::Sender<()>>,
    /// Join handle for the MCU mix loop task.
    pub mcu_loop_handle: Option<tokio::task::JoinHandle<()>>,
    /// Shared mute flag — send loop checks this to skip encoding.
    pub muted_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Shared deafen flag — receive loop checks this to send silence.
    pub deafened_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Handle to the DHT record manager.
///
/// Wraps the protocol crate's `DHTManager` (which holds a `RoutingContext`)
/// and adds a friend-key mapping for presence tracking.
pub struct DHTManagerHandle {
    /// The real DHT manager from the protocol crate.
    pub manager: rekindle_protocol::dht::DHTManager,
    /// Mapping of DHT record keys to the friend public keys that own them.
    /// Populated when watching friend presence records.
    pub dht_key_to_friend: HashMap<String, String>,
    /// Mapping of conversation DHT record keys to friend public keys.
    /// Used to route conversation record change events to the right friend.
    pub conversation_key_to_friend: HashMap<String, String>,
    /// All DHT record keys opened during this session.
    /// Closed in bulk during `shutdown_node` to prevent stale records
    /// in Veilid's `table_store` causing "record already exists" on restart.
    pub open_records: HashSet<String>,
}

impl DHTManagerHandle {
    /// Create a new DHT manager backed by the given routing context.
    pub fn new(routing_context: veilid_core::RoutingContext) -> Self {
        Self {
            manager: rekindle_protocol::dht::DHTManager::new(routing_context),
            dht_key_to_friend: HashMap::new(),
            conversation_key_to_friend: HashMap::new(),
            open_records: HashSet::new(),
        }
    }

    /// Track a DHT record key as opened in this session.
    pub fn track_open_record(&mut self, key: String) {
        self.open_records.insert(key);
    }

    /// Remove a record from tracking (already closed).
    pub fn untrack_record(&mut self, key: &str) {
        self.open_records.remove(key);
    }

    /// Register a friend's DHT record key for presence tracking.
    pub fn register_friend_dht_key(&mut self, dht_key: String, friend_public_key: String) {
        self.dht_key_to_friend.insert(dht_key, friend_public_key);
    }

    /// Look up which friend owns a given DHT key.
    pub fn friend_for_dht_key(&self, dht_key: &str) -> Option<&String> {
        self.dht_key_to_friend.get(dht_key)
    }

    /// Remove a friend's DHT key mapping.
    pub fn unregister_friend_dht_key(&mut self, dht_key: &str) {
        self.dht_key_to_friend.remove(dht_key);
    }

    /// Register a conversation DHT record key → friend public key mapping.
    pub fn register_conversation_key(
        &mut self,
        conversation_key: String,
        friend_public_key: String,
    ) {
        self.conversation_key_to_friend
            .insert(conversation_key, friend_public_key);
    }

    /// Look up which friend owns a given conversation DHT key.
    pub fn friend_for_conversation_key(&self, conversation_key: &str) -> Option<&String> {
        self.conversation_key_to_friend.get(conversation_key)
    }

    /// Remove a conversation key mapping.
    pub fn unregister_conversation_key(&mut self, conversation_key: &str) {
        self.conversation_key_to_friend.remove(conversation_key);
    }

    /// Set the profile DHT key on the inner manager and track the record.
    pub fn set_profile_key(&mut self, key: &str) {
        self.manager.profile_key = Some(key.to_string());
        self.track_open_record(key.to_string());
    }

    /// Set the friend list DHT key on the inner manager and track the record.
    pub fn set_friend_list_key(&mut self, key: &str) {
        self.manager.friend_list_key = Some(key.to_string());
        self.track_open_record(key.to_string());
    }
}

/// Handle to the Veilid routing manager (private route lifecycle).
pub struct RoutingManagerHandle {
    /// The real routing manager from the protocol crate.
    pub manager: rekindle_protocol::routing::RoutingManager,
    /// Timestamped peer route blobs for staleness-aware eviction.
    pub peer_route_cache: rekindle_route::cache::RouteCache,
    /// Shared private-route refresh lifecycle for dead-route recovery and cadence checks.
    pub route_lifecycle: rekindle_route::lifecycle::RouteLifecycle,
}

/// The logged-in user's identity state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityState {
    pub public_key: String,
    pub display_name: String,
    pub status: UserStatus,
    pub status_message: String,
}

/// Online status enum matching Xfire's status system.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    #[default]
    Online,
    Away,
    Busy,
    Offline,
    /// Invisible: the user is online but appears offline to others.
    /// They can still receive messages and see who's online.
    Invisible,
}

/// Whether a friendship is pending (outbound request sent) or fully accepted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FriendshipState {
    PendingOut,
    #[default]
    Accepted,
    /// Friend removal in progress — kept in state so `sync_service` can retry
    /// the `Unfriended` notification using the friend's routing info.
    /// Hidden from the UI via `get_friends` filtering.
    Removing,
}

/// A friend's state as seen on the buddy list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendState {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub status: UserStatus,
    pub status_message: Option<String>,
    pub game_info: Option<GameInfoState>,
    pub group: Option<String>,
    pub unread_count: u32,
    /// The friend's DHT profile record key for presence watching.
    pub dht_record_key: Option<String>,
    /// Unix timestamp (ms) of when this friend was last seen online.
    pub last_seen_at: Option<i64>,
    /// Our local conversation DHT record key for this friend.
    pub local_conversation_key: Option<String>,
    /// The friend's conversation DHT record key (their side).
    pub remote_conversation_key: Option<String>,
    /// The friend's mailbox DHT key (for route discovery).
    pub mailbox_dht_key: Option<String>,
    /// Unix timestamp (ms) from the friend's last DHT heartbeat.
    /// Used for stale presence detection.
    pub last_heartbeat_at: Option<i64>,
    /// Whether this friendship is pending (request sent) or fully accepted.
    pub friendship_state: FriendshipState,
}

/// Game presence information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoState {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    /// Direct server address ("ip:port") for join-game functionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}

/// A joined community's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityState {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channels: Vec<ChannelInfo>,
    pub categories: Vec<CategoryInfo>,
    /// Our role IDs in this community (multi-role, bitmask-based).
    pub my_role_ids: Vec<u32>,
    /// Cached role definitions from merged governance state.
    pub roles: Vec<RoleDefinition>,
    /// Owner keypair for the community DHT record (Veilid `KeyPair::to_string()` format).
    /// Required to open the record with write access.
    pub dht_owner_keypair: Option<String>,
    /// Our pseudonym pubkey hex for this community.
    pub my_pseudonym_key: Option<String>,
    /// Current MEK generation we have.
    pub mek_generation: u64,
    /// DHT member registry record key (SMPL, multi-writer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_registry_key: Option<String>,
    /// Our subkey index in the member registry SMPL record.
    /// **Local** to our segment's record (0..255). The corresponding global
    /// slot index used for slot-keypair derivation is
    /// `my_segment_index * 255 + my_subkey_index` (architecture §15.2 +
    /// §8.3). For segment 0 these are equal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_subkey_index: Option<u32>,
    /// Plate Gate (architecture §15): which segment hosts our slot. `None`
    /// for legacy / unknown; 0 for the genesis segment; 1..=MAX_SEGMENTS
    /// for each expansion. Persisted to SQLite, restored on login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_segment_index: Option<u32>,
    // ── v2.0 flat governance fields ──
    /// DHT key of the SMPL governance record (o_cnt:0).
    /// This is the canonical community identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance_key: Option<String>,

    /// Cached CRDT-merged governance state.
    /// Computed by `rekindle_governance::merge::merge()`.
    #[serde(skip)]
    pub governance_state: Option<rekindle_governance::state::GovernanceState>,

    /// Per-community Lamport counter for deterministic message ordering.
    /// Incremented on every send, merged with max(local, received)+1 on receive.
    #[serde(skip)]
    pub lamport_counter: u64,

    // ── Gossip mesh fields (Phase 2) ──
    /// Gossip overlay state (peer set, online members, lamport counter).
    /// `None` until the presence poll loop initializes it.
    #[serde(skip)]
    pub gossip: Option<GossipOverlay>,

    /// Our slot keypair string for writing presence to the SMPL registry.
    /// Veilid `KeyPair::to_string()` format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_keypair: Option<String>,

    /// Channel DHTLog record keys: channel_id → log spine key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_log_keys: HashMap<String, String>,

    /// Registry segment owner keypair for writing member index/MEK vault.
    /// Veilid `KeyPair::to_string()` format. Required to add/remove members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_owner_keypair: Option<String>,

    /// Slot seed for deriving member SMPL keypairs.
    /// Distributed to ALL members via JoinAccepted (same trust level as MEK).
    /// 32 bytes, hex-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_seed: Option<String>,

    /// In-memory cache of known member pseudonym keys.
    /// Populated from SQLite on login, updated on MemberJoined/MemberRemoved/MemberLeave.
    /// Used for fast membership checks on incoming community gossip envelopes.
    #[serde(skip)]
    pub known_members: HashSet<String>,

    /// Cached member role_ids: pseudonym_key → role_ids.
    /// Populated during presence_poll_tick from the member index.
    /// Used for permission checks on incoming gossip moderation payloads.
    #[serde(skip)]
    pub member_roles: HashMap<String, Vec<u32>>,

    /// Per-channel message sequence counter (channel_id → next sequence).
    /// Incremented for each message we send in a channel. Persisted to SQLite.
    #[serde(skip)]
    pub channel_sequences: HashMap<String, u64>,

    /// Pending sync requests: channel_id → (request_timestamp, attempt_count).
    /// Cleared when SyncResponse arrives. Retried in presence_poll_tick if stale.
    #[serde(skip)]
    pub pending_syncs: HashMap<String, (u64, u32)>,

    /// Record keys with an active Veilid watch in this session.
    /// Watches are an optimization; inspect polling still runs for all records.
    #[serde(skip)]
    pub watched_records: HashSet<String>,

    /// Last known network sequence numbers per record key, used by the 60-second
    /// inspect loop to detect changed subkeys without fetching the entire record.
    #[serde(skip)]
    pub record_sequences: HashMap<String, Vec<veilid_core::ValueSeqNum>>,

    /// Per-sender per-channel sequence tracking for gap detection (Briar-inspired).
    /// Key: (sender_pseudonym, channel_id), Value: last received sequence number.
    #[serde(skip)]
    pub peer_sequences: HashMap<(String, String), u64>,

    /// Architecture §28.7 slowmode: per-channel timestamp (ms) of our
    /// last successful send. Compared against the channel's
    /// `slowmode_seconds` to gate further writes. In-memory only —
    /// slowmode applies to the active session, not across restarts.
    #[serde(skip)]
    pub channel_last_send_at: HashMap<String, i64>,

    /// Mutual Aid topology metrics (architecture §14.5): per-peer
    /// (success_count, failure_count) over the lifetime of this session.
    /// Used to weight gossip fan-out selection — the highest-reliability
    /// peers ("ziplines") emerge organically from usage. Pure in-memory;
    /// not persisted across restarts.
    #[serde(skip)]
    pub peer_reliability: HashMap<String, (u32, u32)>,

    /// Shutdown sender for the presence poll loop.
    #[serde(skip)]
    pub presence_poll_shutdown_tx: Option<mpsc::Sender<()>>,

    /// Shutdown sender for the DHT keepalive loop.
    #[serde(skip)]
    pub dht_keepalive_shutdown_tx: Option<mpsc::Sender<()>>,

    /// Tracks all DHT records opened for this community (VeilidChat-inspired lifecycle).
    /// Records are opened once during join and kept open until leave/logout.
    /// Prevents "record not open" errors and ensures proper cleanup.
    #[serde(skip)]
    pub open_community_records: CommunityRecords,

    /// Our locally persisted RSVPs for scheduled events.
    #[serde(skip)]
    pub my_event_rsvps: HashMap<String, String>,

    /// Reader-aggregated RSVPs discovered from member presence records.
    #[serde(skip)]
    pub event_rsvps_by_event: HashMap<String, Vec<EventRsvpEntry>>,

    /// Whether our local member record has completed onboarding for this community.
    #[serde(skip)]
    pub onboarding_complete: bool,

    /// Per-community profile bio (≤190 chars). Same identity, different
    /// persona per community — the value is propagated to peers via the next
    /// presence write. Local-only state; on restart it resets to None and
    /// the user re-edits it from the popup.
    #[serde(skip)]
    pub my_bio: Option<String>,

    /// Per-community profile pronouns (≤40 chars per architecture §24.2).
    /// See `my_bio`.
    #[serde(skip)]
    pub my_pronouns: Option<String>,

    /// Per-community profile theme color (0xRRGGBB). See `my_bio`.
    #[serde(skip)]
    pub my_theme_color: Option<u32>,

    /// Per-community profile badges (≤8 entries, each ≤32 chars). See `my_bio`.
    #[serde(skip)]
    pub my_badges: Vec<String>,

    /// Per-community avatar content reference (BLAKE3 hex hash of the
    /// image stored as a Lost Cargo expression asset). Architecture
    /// §24.2 + §32 Week 15.
    #[serde(skip)]
    pub my_avatar_ref: Option<String>,

    /// Per-community banner content reference (BLAKE3 hex hash). Same
    /// caching model as `my_avatar_ref`.
    #[serde(skip)]
    pub my_banner_ref: Option<String>,

    /// Architecture §32 Phase 5 Week 15 — community-level icon (the
    /// avatar shown in the buddy list / community switcher). BLAKE3
    /// hex hash of the WebP-compressed image cached at
    /// `<app_data>/community_avatars/<community_id>/<hash>.webp`.
    /// Synced from `governance.metadata.icon_hash` when CRDT state
    /// is rebuilt; persisted to the `communities.icon_hash` column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_hash: Option<String>,

    /// Architecture §32 Phase 5 Week 15 — community-level banner.
    /// Same caching model as `icon_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_hash: Option<String>,

    /// Reader-aggregated profile snapshots (peer pseudonym → profile fields).
    /// Populated by `presence_poll_tick` from peers' presence subkeys.
    #[serde(skip)]
    pub member_profiles: HashMap<String, MemberProfileSnapshot>,

    /// Architecture §20.6 raid detector — sliding window of recent
    /// (timestamp_secs, pseudonym_hex) join observations. Bounded to the
    /// policy window length when entries are inserted.
    #[serde(skip)]
    pub recent_member_joins: std::collections::VecDeque<(u64, String)>,
}

/// Tracks DHT records opened for a single community.
///
/// Follows VeilidChat's "open once, keep open" pattern: records are opened during
/// join_community and closed only on leave or logout. Presence poll and keepalive
/// use the already-open records via `get_dht_value` without re-opening.
#[derive(Debug, Default, Clone)]
pub struct CommunityRecords {
    /// The primary community governance record key.
    pub governance_key: Option<String>,
    /// The SMPL member registry record key.
    pub registry_key: Option<String>,
    /// Writer keypair used when opening the registry (preserved to avoid clobber on re-open).
    pub registry_writer: Option<String>,
    /// All opened channel SMPL record keys.
    pub channel_keys: Vec<String>,
    /// Whether records have been opened for this session (false after restart until rejoin).
    pub records_open: bool,
    /// Fingerprint of the last inspected governance record state.
    pub governance_report_fingerprint: Option<u64>,
    /// Fingerprints of the last inspected channel record state by channel id.
    pub channel_report_fingerprints: HashMap<String, u64>,
}

/// Aggregated RSVP entry for a single member and event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRsvpEntry {
    pub pseudonym_key: String,
    pub status: String,
}

/// Per-community profile snapshot aggregated from a peer's presence subkey.
///
/// The presence poll (`presence/poll.rs::presence_poll_tick`) writes one
/// entry per discovered member into `CommunityState.member_profiles`.
/// `get_community_members` joins this map with the SQLite membership rows so
/// the popup can render `bio` / `pronouns` / `theme_color` / `badges`.
#[derive(Debug, Clone, Default)]
pub struct MemberProfileSnapshot {
    /// Mirror of `MemberPresence.display_name` so mention-resolution
    /// (architecture §28.5) can map `@name` → pseudonym hex without a
    /// SQLite round-trip on every send.
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// A role definition cached from merged governance state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDefinition {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    /// Architecture §19.4 — when set, the member can hold at most one
    /// role per group (CRDT-enforced; the higher-Lamport assignment
    /// wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_group: Option<String>,
}

impl RoleDefinition {
    /// Convert from the protocol's `RoleDto`.
    pub fn from_dto(dto: &rekindle_protocol::messaging::RoleDto) -> Self {
        Self {
            id: dto.id,
            name: dto.name.clone(),
            color: dto.color,
            permissions: dto.permissions,
            position: dto.position,
            hoist: dto.hoist,
            mentionable: dto.mentionable,
            self_assignable: dto.self_assignable,
            exclusion_group: None,
        }
    }
}

/// Compute the display name for the highest-positioned role from a set of role IDs.
pub fn display_role_name(role_ids: &[u32], roles: &[RoleDefinition]) -> String {
    match role_ids
        .iter()
        .filter_map(|id| roles.iter().find(|r| r.id == *id))
        .max_by_key(|r| r.position)
    {
        Some(r) => r.name.clone(),
        None => "member".to_string(),
    }
}

/// Channel info within a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub unread_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default)]
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forum_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_speakers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_moderator: Option<String>,
    #[serde(default)]
    pub slowmode_seconds: Option<u32>,
    /// Whether this channel is marked NSFW.
    #[serde(default)]
    pub nsfw: bool,
    /// DHT record key for this channel's per-channel message record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_record_key: Option<String>,
    /// Current MEK generation for this channel.
    #[serde(default)]
    pub mek_generation: u64,
    /// Local notification preference for this channel.
    #[serde(default = "default_notification_level")]
    pub notification_level: String,
    /// Architecture §32 Phase 7 Week 25 — channel-level notification
    /// sound override (BLAKE3 content hash of a soundboard expression).
    /// `None` means inherit from the community default; the resolver
    /// in `services/community/notifications.rs::resolve_notification_sound`
    /// performs the channel → community-default → app-default cascade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_sound_ref: Option<String>,
    /// Architecture §10.8 — text-in-voice. Hex channel id of the
    /// parent voice channel when this channel is the text companion of
    /// a voice channel; `None` for normal channels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_voice_channel_id: Option<String>,
}

fn default_notification_level() -> String {
    "all".to_string()
}

/// Category info within a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Text,
    Voice,
    Announcement,
    Forum,
    Stage,
    Directory,
    Media,
    Events,
    Dm,
}

impl AsRef<str> for ChannelType {
    fn as_ref(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Announcement => "announcement",
            Self::Forum => "forum",
            Self::Stage => "stage",
            Self::Directory => "directory",
            Self::Media => "media",
            Self::Events => "events",
            Self::Dm => "dm",
        }
    }
}

impl From<rekindle_protocol::dht::community::types::ChannelKind> for ChannelType {
    fn from(kind: rekindle_protocol::dht::community::types::ChannelKind) -> Self {
        use rekindle_protocol::dht::community::types::ChannelKind;
        match kind {
            ChannelKind::Text => Self::Text,
            ChannelKind::Voice => Self::Voice,
            ChannelKind::Announcement => Self::Announcement,
            ChannelKind::Forum => Self::Forum,
            ChannelKind::Stage => Self::Stage,
            ChannelKind::Directory => Self::Directory,
            ChannelKind::Media => Self::Media,
            ChannelKind::Events => Self::Events,
            ChannelKind::Dm => Self::Dm,
        }
    }
}

impl fmt::Display for ChannelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl FromStr for ChannelType {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "voice" => Self::Voice,
            "announcement" => Self::Announcement,
            "forum" => Self::Forum,
            "stage" => Self::Stage,
            "directory" => Self::Directory,
            "media" => Self::Media,
            "events" => Self::Events,
            "dm" => Self::Dm,
            _ => Self::Text,
        })
    }
}

impl rusqlite::types::ToSql for ChannelType {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::Borrowed(
            rusqlite::types::ValueRef::Text(self.as_ref().as_bytes()),
        ))
    }
}

impl rusqlite::types::FromSql for ChannelType {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let s = value.as_str()?;
        Ok(s.parse().unwrap_or(Self::Text))
    }
}
