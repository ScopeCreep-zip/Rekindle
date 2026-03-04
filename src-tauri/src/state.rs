use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
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
    /// MEK cache: `community_id` -> current `MediaEncryptionKey` (decrypted from server).
    /// Legacy community-level MEK — used during the transition to per-channel MEK.
    pub mek_cache: Mutex<HashMap<String, MediaEncryptionKey>>,
    /// Per-channel MEK cache: `(community_id, channel_id)` -> `MediaEncryptionKey`.
    /// New per-channel MEK distribution: each channel has its own encryption key.
    pub channel_mek_cache: Mutex<HashMap<(String, String), MediaEncryptionKey>>,
    /// Per-community coordinator service handles.
    /// Key: community_id, Value: handle for querying role + forwarding value changes.
    pub coordinator_services: RwLock<HashMap<String, crate::services::coordinator::CoordinatorServiceHandle>>,
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
    /// (coordinator relay, heartbeat, etc.) to emit events and access managed state.
    pub app_handle: RwLock<Option<tauri::AppHandle>>,
    /// Global dedup cache for gossip mesh message deduplication.
    /// Prevents processing/forwarding the same message twice.
    pub dedup_cache: Mutex<DedupCache>,
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
            coordinator_services: RwLock::new(HashMap::new()),
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
        }
    }
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
#[derive(Debug, Clone)]
pub struct GossipOverlay {
    /// Current gossip peers: pseudonym_key → route_blob.
    /// These are the D peers we send/forward every message to.
    pub peers: HashMap<String, Vec<u8>>,
    /// All online members: pseudonym_key → route_blob.
    /// Superset of `peers`. Updated on each presence poll.
    pub online_members: HashMap<String, Vec<u8>>,
    /// Lamport counter for outgoing messages.
    /// Incremented for each message we originate (not forwards).
    pub lamport_counter: u64,
    /// True until the first successful sync after coming online.
    /// Used to trigger a `SyncRequest` to online peers for catch-up.
    pub needs_initial_sync: bool,
}

impl Default for GossipOverlay {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            online_members: HashMap::new(),
            lamport_counter: 0,
            needs_initial_sync: true,
        }
    }
}

/// Compute the gossip degree (D) for the given number of online members.
///
/// - ≤1: no gossip (alone)
/// - 2-20: direct mesh (send to everyone)
/// - 21-60: D=6
/// - 61+: D=8
pub fn gossip_degree(online_count: usize) -> usize {
    match online_count {
        0..=1 => 0,
        2..=20 => online_count - 1,
        21..=60 => 6,
        _ => 8,
    }
}

/// LRU message deduplication cache.
///
/// Prevents infinite gossip forwarding loops and duplicate processing.
/// Key = `(community_id, sender_pseudonym, dedup_key)`.
/// FIFO eviction when capacity exceeded.
pub struct DedupCache {
    entries: indexmap::IndexMap<(String, String, String), ()>,
    capacity: usize,
}

impl DedupCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: indexmap::IndexMap::with_capacity(capacity),
            capacity,
        }
    }

    /// Remove all entries from the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns `true` if the message is a duplicate (already seen).
    /// If new, inserts it and evicts the oldest entry if at capacity.
    pub fn check_and_insert(&mut self, community_id: &str, sender: &str, dedup_key: &str) -> bool {
        let key = (
            community_id.to_string(),
            sender.to_string(),
            dedup_key.to_string(),
        );
        if self.entries.contains_key(&key) {
            return true;
        }
        if self.entries.len() >= self.capacity {
            self.entries.shift_remove_index(0);
        }
        self.entries.insert(key, ());
        false
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
    /// Cached role definitions from the server.
    pub roles: Vec<RoleDefinition>,
    /// Display string for our highest role (for backward-compat display).
    pub my_role: Option<String>,
    /// The DHT record key for this community's shared state.
    pub dht_record_key: Option<String>,
    /// Owner keypair for the community DHT record (Veilid `KeyPair::to_string()` format).
    /// Required for the server to open the record with write access.
    pub dht_owner_keypair: Option<String>,
    /// Our pseudonym pubkey hex for this community.
    pub my_pseudonym_key: Option<String>,
    /// Current MEK generation we have.
    pub mek_generation: u64,
    /// DHT manifest record key (DFLT, 16 subkeys).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_key: Option<String>,
    /// DHT member registry record key (SMPL, multi-writer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_registry_key: Option<String>,
    /// Our subkey index in the member registry SMPL record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_subkey_index: Option<u32>,
    /// Coordinator's pseudonym public key hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator_pseudonym: Option<String>,
    /// Coordinator's route blob for sending envelopes to the active coordinator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator_route_blob: Option<Vec<u8>>,
    /// Coordinator epoch — incremented on coordinator restart.
    #[serde(default)]
    pub coordinator_epoch: u64,

    // ── Gossip mesh fields (Phase 2) ──

    /// Gossip overlay state (peer set, online members, lamport counter).
    /// `None` until the presence poll loop initializes it.
    #[serde(skip)]
    pub gossip: Option<GossipOverlay>,

    /// Our slot keypair string for writing presence to the SMPL registry.
    /// Veilid `KeyPair::to_string()` format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_keypair: Option<String>,

    /// Manifest owner keypair (shared with admins for write access).
    /// Veilid `KeyPair::to_string()` format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_owner_keypair: Option<String>,

    /// Channel DHTLog record keys: channel_id → log spine key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_log_keys: HashMap<String, String>,

    /// Registry segment owner keypair for writing member index/MEK vault.
    /// Veilid `KeyPair::to_string()` format. Required to add/remove members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_owner_keypair: Option<String>,

    /// Slot seed for deriving member SMPL keypairs (admins only).
    /// 32 bytes, hex-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_seed: Option<String>,

    /// In-memory cache of known member pseudonym keys.
    /// Populated from SQLite on login, updated on MemberJoined/MemberRemoved/MemberLeave.
    /// Used for fast membership checks on incoming ChatMessages.
    #[serde(skip)]
    pub known_members: HashSet<String>,

    /// Shutdown sender for the presence poll loop.
    #[serde(skip)]
    pub presence_poll_shutdown_tx: Option<mpsc::Sender<()>>,

    /// Shutdown sender for the DHT keepalive loop.
    #[serde(skip)]
    pub dht_keepalive_shutdown_tx: Option<mpsc::Sender<()>>,
}

/// A role definition cached from the server.
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
}

impl AsRef<str> for ChannelType {
    fn as_ref(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Announcement => "announcement",
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

/// Parse a DHT channel list blob into `Vec<ChannelInfo>`.
///
/// Supports both the wrapped format `{ "channels": [...] }` and a bare JSON array `[...]`.
pub(crate) fn parse_dht_channel_list(data: &[u8]) -> Vec<ChannelInfo> {
    let channel_list: Vec<serde_json::Value> =
        match serde_json::from_slice::<serde_json::Value>(data) {
            Ok(v) => {
                if let Some(obj) = v.as_object() {
                    obj.get("channels")
                        .and_then(|c| c.as_array().cloned())
                        .unwrap_or_default()
                } else {
                    v.as_array().cloned().unwrap_or_default()
                }
            }
            Err(_) => return vec![],
        };
    channel_list
        .iter()
        .filter_map(|ch| {
            let id = ch.get("id")?.as_str()?.to_string();
            let name = ch.get("name")?.as_str()?.to_string();
            let ch_type: ChannelType = ch
                .get("channelType")
                .and_then(|v| v.as_str())
                .unwrap_or("text")
                .parse()
                .unwrap_or(ChannelType::Text);
            let category_id = ch
                .get("categoryId")
                .and_then(|v| v.as_str())
                .map(String::from);
            let topic = ch
                .get("topic")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let slowmode_seconds = ch
                .get("slowmodeSeconds")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            Some(ChannelInfo {
                id,
                name,
                channel_type: ch_type,
                unread_count: 0,
                category_id,
                topic,
                slowmode_seconds,
                nsfw: false,
                message_record_key: None,
                mek_generation: 0,
            })
        })
        .collect()
}

