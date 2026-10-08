//! Phase 23.B — extracted from `state.rs`. Veilid + Signal + Voice +
//! Game runtime handles owned by `AppState`. Each is an `Option<T>`
//! field set on the relevant lifecycle hook (login, voice join,
//! game-detect spawn).

use std::collections::HashMap;

use super::friend::GameInfoState;

/// Handle to the Veilid node.
pub struct NodeHandle {
    /// Raw Veilid `AttachmentState` string (e.g. "detached", "attaching", "`attached_good`").
    pub attachment_state: String,
    /// Whether the node is attached to the network.
    pub is_attached: bool,
    /// Whether the public internet is ready for DHT operations.
    pub public_internet_ready: bool,
    /// Reliable peers in the routing table (0.5.7 attachment signal).
    pub reliable_peer_count: u64,
    /// Live peers (reliable + unreliable + newly added) in the routing table.
    pub live_peer_count: u64,
    /// Smoothed estimate of total reachable network size.
    pub estimated_network_size: u64,
    /// Median p75 latency across reliable peers, in microseconds (0 = no samples).
    pub median_latency_us: u64,
    /// Veilid API handle (needed for shutdown, route import, etc.).
    pub api: veilid_core::VeilidAPI,
    /// Veilid routing context (needed for `app_message`, DHT ops).
    pub routing_context: veilid_core::RoutingContext,
    /// Our DHT profile record key.
    pub profile_dht_key: Option<String>,
    /// Owner keypair for our profile DHT record (needed to re-open with write access).
    pub profile_owner_keypair: Option<veilid_core::KeyPair>,
    /// The record pool's session lease on our profile, released when the
    /// profile is rotated to a new record (plan C7.4).
    pub profile_lease: Option<rekindle_records::lease::LeaseId>,
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
    pub fn set_profile_dht(
        &mut self,
        key: String,
        keypair: veilid_core::KeyPair,
        lease: rekindle_records::lease::LeaseId,
    ) {
        self.profile_dht_key = Some(key);
        self.profile_owner_keypair = Some(keypair);
        self.profile_lease = Some(lease);
    }

    /// Store friend list DHT key and owner keypair after publishing.
    pub fn set_friend_list_dht(&mut self, key: String, keypair: veilid_core::KeyPair) {
        self.friend_list_dht_key = Some(key);
        self.friend_list_owner_keypair = Some(keypair);
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
    /// Current detected game info.
    pub current_game: Option<GameInfoState>,
}

/// Handle to the voice engine.
pub struct VoiceEngineHandle {
    pub engine: rekindle_voice::VoiceEngine,
    /// Shared voice transport — used by send loop, MCU loop, and VoiceJoin handler.
    pub transport: std::sync::Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
    /// The transport's media roster: route queues, feedback intake and the
    /// allocator, reachable without the transport's async lock (plan E4.3.3).
    pub media: std::sync::Arc<rekindle_voice::transport::roster::MediaRoster>,
    /// Which channel/call we're currently in (prevents double-joining).
    /// The send and receive loops and the video allocation follower
    /// (plan C4).
    pub loops: Option<std::sync::Arc<rekindle_lifecycle::SessionScope>>,
    /// The device monitor.
    pub monitor: Option<std::sync::Arc<rekindle_lifecycle::SessionScope>>,
    /// The MCU mix loop, while this peer is the voice host.
    pub mcu: Option<std::sync::Arc<rekindle_lifecycle::SessionScope>>,
    pub channel_id: String,
    /// Community ID if this is a community voice channel (None for DM voice).
    pub community_id: Option<String>,
    /// Shared mute flag — send loop checks this to skip encoding.
    pub muted_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Shared deafen flag — receive loop checks this to send silence.
    pub deafened_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Shared media-plane liveness ledger — fed by the receive loop
    /// (accepted voice packets) and the send loop (verified receiver
    /// reports); read by the signaling adapter's `media_live_peers()`
    /// so the presence reconcile never evicts a peer whose media is
    /// flowing.
    pub media_liveness: std::sync::Arc<rekindle_voice::liveness::MediaLiveness>,
}

/// The session's friend record-key maps, for routing value changes to a
/// friend. DHT records themselves are the session's record pool's
/// (plan C7); this handle no longer holds a manager or an open-record set.
#[derive(Default)]
pub struct DHTManagerHandle {
    /// Mapping of DHT record keys to the friend public keys that own them.
    /// Populated when watching friend presence records.
    pub dht_key_to_friend: HashMap<String, String>,
    /// Mapping of conversation DHT record keys to friend public keys.
    /// Used to route conversation record change events to the right friend.
    pub conversation_key_to_friend: HashMap<String, String>,
}

impl DHTManagerHandle {
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
}

/// Handle to the Veilid routing manager (private route lifecycle).
pub struct RoutingManagerHandle {
    /// Timestamped peer route blobs for staleness-aware eviction. Our own
    /// routes are `AppState.own_routes` (plan C7.9b).
    pub peer_route_cache: rekindle_route::cache::RouteCache,
}
