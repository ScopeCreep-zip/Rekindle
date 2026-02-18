use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Wrapper around `std::process::Child` that kills the child process on drop.
///
/// Prevents orphaned child processes when the parent exits unexpectedly (crash,
/// force-quit). On normal shutdown, the process is killed and waited for so
/// no zombie process remains.
pub struct KillOnDropChild(std::process::Child);

impl KillOnDropChild {
    /// Wrap an existing `Child` process.
    pub fn new(child: std::process::Child) -> Self {
        Self(child)
    }

    /// Get the OS-assigned process ID.
    pub fn id(&self) -> u32 {
        self.0.id()
    }

    /// Check if the child has exited without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.0.try_wait()
    }

    /// Send a kill signal to the child process.
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.0.kill()
    }

    /// Block until the child process exits.
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.0.wait()
    }
}

impl Drop for KillOnDropChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

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
    pub mek_cache: Mutex<HashMap<String, MediaEncryptionKey>>,
    /// Community server child process handle (spawned on login if user owns communities).
    /// Wrapped in `KillOnDropChild` so the child is killed if the parent crashes.
    pub server_process: Mutex<Option<KillOnDropChild>>,
    /// Shutdown sender for the server health check loop.
    pub server_health_shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    /// Community server route cache: `community_id` -> imported `RouteId`.
    /// For communities we're a MEMBER of (not owner) — the remote server's route.
    pub community_routes: RwLock<HashMap<String, veilid_core::RouteId>>,
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
            server_process: Mutex::new(None),
            server_health_shutdown_tx: Arc::new(RwLock::new(None)),
            community_routes: RwLock::new(HashMap::new()),
            unwatched_friends: RwLock::new(HashSet::new()),
            dispatch_loop_handle: RwLock::new(None),
            route_refresh_shutdown_tx: RwLock::new(None),
            idle_shutdown_tx: RwLock::new(None),
            heartbeat_shutdown_tx: RwLock::new(None),
            pre_away_status: RwLock::new(None),
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
    pub fn register_conversation_key(&mut self, conversation_key: String, friend_public_key: String) {
        self.conversation_key_to_friend.insert(conversation_key, friend_public_key);
    }

    /// Look up which friend owns a given conversation DHT key.
    pub fn friend_for_conversation_key(&self, conversation_key: &str) -> Option<&String> {
        self.conversation_key_to_friend.get(conversation_key)
    }

    /// Remove a conversation key mapping.
    pub fn unregister_conversation_key(&mut self, conversation_key: &str) {
        self.conversation_key_to_friend.remove(conversation_key);
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
}

/// Whether a friendship is pending (outbound request sent) or fully accepted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FriendshipState {
    PendingOut,
    #[default]
    Accepted,
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
}

/// A joined community's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityState {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channels: Vec<ChannelInfo>,
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
    /// Community server's private route blob (for sending `app_call`).
    pub server_route_blob: Option<Vec<u8>>,
    /// Whether we host this community's server process.
    pub is_hosted: bool,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Text,
    Voice,
}
