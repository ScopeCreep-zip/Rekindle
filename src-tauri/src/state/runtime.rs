//! Phase 23.B — extracted from `state.rs`. Veilid + Voice
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
