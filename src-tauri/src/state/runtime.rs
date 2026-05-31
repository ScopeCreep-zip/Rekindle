//! Phase 23.B — extracted from `state.rs`. Veilid + Signal + Voice +
//! Game runtime handles owned by `AppState`. Each is an `Option<T>`
//! field set on the relevant lifecycle hook (login, voice join,
//! game-detect spawn).

use std::collections::{HashMap, HashSet};

use tokio::sync::mpsc;

use super::friend::GameInfoState;

/// Handle to the Veilid node.
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
