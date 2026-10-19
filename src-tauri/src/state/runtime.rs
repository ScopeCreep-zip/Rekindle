//! Phase 23.B — extracted from `state.rs`. Veilid + Signal + Voice
//! runtime handles owned by `AppState`. Each is an `Option<T>` field set on
//! the relevant lifecycle hook (node start, login, voice join).

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
    /// Our DHT friend list record key.
    pub friend_list_dht_key: Option<String>,
    /// Our mailbox DHT record key (deterministic, permanent).
    pub mailbox_dht_key: Option<String>,
}

/// Handle to the Signal session manager.
pub struct SignalManagerHandle {
    /// The unlock's one Signal manager, shared with the in-process host
    /// (plan D3.1), so a bus request and the window use one session state.
    pub manager: std::sync::Arc<rekindle_crypto::SignalSessionManager>,
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
