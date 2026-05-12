//! Transport configuration types for the IPC boundary.
//!
//! These structs are constructed by CLI config loading and passed to the
//! node daemon at startup. Defined here in `rekindle-types` so that both
//! the CLI (config producer) and the node/transport (config consumer) can
//! use them without the CLI depending on `rekindle-transport`.

use serde::{Deserialize, Serialize};

/// Top-level transport configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// Base storage directory for Veilid persistent state.
    pub storage_dir: String,

    /// Application namespace on the Veilid network.
    #[serde(default = "default_namespace")]
    pub namespace: String,

    /// Per-data-class safety routing parameters.
    #[serde(default)]
    pub safety: SafetyConfig,

    /// RPC timeout in milliseconds for app_call operations.
    #[serde(default = "default_rpc_timeout_ms")]
    pub rpc_timeout_ms: u64,

    /// Maximum retry attempts for failed DHT writes.
    #[serde(default = "default_dht_write_retries")]
    pub dht_write_retries: u32,

    /// Interval in seconds between route refresh cycles.
    #[serde(default = "default_route_refresh_secs")]
    pub route_refresh_secs: u64,

    /// TTL in seconds for cached imported routes before re-import.
    #[serde(default = "default_route_cache_ttl_secs")]
    pub route_cache_ttl_secs: u64,

    /// Circuit breaker: consecutive failures before tripping.
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,

    /// Circuit breaker: cooldown period in seconds after tripping.
    #[serde(default = "default_circuit_breaker_cooldown_secs")]
    pub circuit_breaker_cooldown_secs: u64,

    /// Gossip dedup cache capacity (number of entries).
    #[serde(default = "default_dedup_cache_capacity")]
    pub dedup_cache_capacity: usize,

    /// Gossip message TTL (max forwarding hops).
    #[serde(default = "default_gossip_ttl")]
    pub gossip_ttl: u8,

    /// Allow Veilid to fall back to its encrypted file-based protected store
    /// when the OS Secret Service (dbus org.freedesktop.secrets) is unavailable.
    ///
    /// Default: false. Set to true in environments without gnome-keyring or
    /// kwallet (containers, headless servers, CI).
    #[serde(default)]
    pub allow_insecure_protected_store: bool,

    /// Port for the Prometheus metrics HTTP endpoint (127.0.0.1 only).
    /// Default: 9191. Set to 0 to disable.
    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,

    /// Port for the health check TCP endpoint (127.0.0.1 only).
    /// Default: 9192. Set to 0 to disable.
    #[serde(default = "default_health_port")]
    pub health_port: u16,

    /// Veilid network configuration. Controls all Veilid-level tunables:
    /// protocol ports, connection limits, DHT parameters, routing table
    /// thresholds, UPNP, and more.
    #[serde(default)]
    pub veilid: VeilidNetworkConfig,
}

// ── Veilid network configuration ───────────────────────────────────────────

/// Complete Veilid network configuration.
///
/// Mirrors every field in `veilid_core::VeilidConfigNetwork` and its
/// sub-structs. All defaults match Veilid's own defaults so existing
/// deployments see zero behavior change unless a field is explicitly set.
///
/// Mapped onto `veilid_core::VeilidConfig` in `TransportNode::start()`
/// before `api_startup` is called.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VeilidNetworkConfig {
    // ── Protocol: TCP ───────────────────────────────────────────────
    /// TCP listen address. Empty = Veilid default (port 5150, search upward).
    /// Format: ":5150" (all interfaces), "0.0.0.0:5150", "192.168.1.1:5150".
    /// Use a port in the kernel ephemeral range (32768-60999) under Landlock.
    #[serde(default)]
    pub tcp_listen_address: String,

    /// Static public TCP address advertised to peers. None = auto-detect.
    /// Set to "1.2.3.4:5150" on cloud VMs with a known public IP to skip
    /// dial-info validation and reach PublicInternet class immediately.
    #[serde(default)]
    pub tcp_public_address: Option<String>,

    /// Enable inbound TCP connections. Default: true.
    #[serde(default = "default_true")]
    pub tcp_listen: bool,

    /// Enable outbound TCP connections. Default: true.
    #[serde(default = "default_true")]
    pub tcp_connect: bool,

    /// Maximum simultaneous TCP connections. Default: 32.
    #[serde(default = "default_tcp_max_connections")]
    pub tcp_max_connections: u32,

    // ── Protocol: UDP ───────────────────────────────────────────────
    /// UDP listen address. Empty = Veilid default (port 5150, search upward).
    #[serde(default)]
    pub udp_listen_address: String,

    /// Static public UDP address advertised to peers. None = auto-detect.
    #[serde(default)]
    pub udp_public_address: Option<String>,

    /// Enable UDP transport. Default: true.
    #[serde(default = "default_true")]
    pub udp_enabled: bool,

    /// UDP socket pool size. 0 = automatic. Default: 0.
    #[serde(default)]
    pub udp_socket_pool_size: u32,

    // ── Protocol: WebSocket ─────────────────────────────────────────
    /// WebSocket listen address. Empty = Veilid default (port 5150).
    #[serde(default)]
    pub ws_listen_address: String,

    /// Enable inbound WebSocket connections. Default: true.
    /// Disable when browser/web clients are not needed.
    #[serde(default = "default_true")]
    pub ws_listen: bool,

    /// Enable outbound WebSocket connections. Default: true.
    #[serde(default = "default_true")]
    pub ws_connect: bool,

    /// Maximum simultaneous WebSocket connections. Default: 32.
    #[serde(default = "default_ws_max_connections")]
    pub ws_max_connections: u32,

    /// WebSocket URL path. Default: "ws".
    #[serde(default = "default_ws_path")]
    pub ws_path: String,

    // ── Connection limits ───────────────────────────────────────────
    /// Maximum connections per IPv4 address. Default: 32.
    #[serde(default = "default_max_connections_per_ip4")]
    pub max_connections_per_ip4: u32,

    /// Maximum connections per IPv6 /prefix address block. Default: 32.
    #[serde(default = "default_max_connections_per_ip6_prefix")]
    pub max_connections_per_ip6_prefix: u32,

    /// IPv6 prefix size for connection grouping. Default: 56.
    #[serde(default = "default_max_connections_per_ip6_prefix_size")]
    pub max_connections_per_ip6_prefix_size: u32,

    /// Maximum new connections per minute from a single IP. Default: 128.
    #[serde(default = "default_max_connection_frequency_per_min")]
    pub max_connection_frequency_per_min: u32,

    /// Allowlist timeout for known client addresses in milliseconds. Default: 300000 (5 min).
    #[serde(default = "default_client_allowlist_timeout_ms")]
    pub client_allowlist_timeout_ms: u32,

    /// Timeout for reverse connection receipts in milliseconds. Default: 5000.
    #[serde(default = "default_reverse_connection_receipt_time_ms")]
    pub reverse_connection_receipt_time_ms: u32,

    /// Timeout for hole-punch receipts in milliseconds. Default: 5000.
    #[serde(default = "default_hole_punch_receipt_time_ms")]
    pub hole_punch_receipt_time_ms: u32,

    // ── Connection timeouts ─────────────────────────────────────────
    /// Timeout for initial connection establishment in milliseconds. Default: 2000.
    #[serde(default = "default_connection_initial_timeout_ms")]
    pub connection_initial_timeout_ms: u32,

    /// Timeout for inactive connections in milliseconds. Default: 60000.
    #[serde(default = "default_connection_inactivity_timeout_ms")]
    pub connection_inactivity_timeout_ms: u32,

    // ── NAT / address detection ─────────────────────────────────────
    /// Enable UPnP for automatic port forwarding. Default: true.
    /// Disable in containers, VMs with static port mapping, or corporate NAT.
    #[serde(default = "default_true")]
    pub upnp: bool,

    /// Monitor and react to network interface address changes. Default: true.
    /// `None` means auto: Veilid enables this when no globally-routable
    /// interface address is found (the right behavior for most deployments).
    #[serde(default = "default_detect_address_changes")]
    pub detect_address_changes: Option<bool>,

    /// Retries for restricted NAT traversal. Default: 0 (disabled).
    #[serde(default)]
    pub restricted_nat_retries: u32,

    /// Require inbound relay for all inbound connections. Default: false.
    ///
    /// Set true when the node is behind NAT with no direct inbound reachability
    /// (Docker bridge, cloud NAT, CGNAT). Veilid will skip dial-info validation
    /// (which requires remote bootstrap peers to connect inbound) and use relays
    /// instead. Without this, NAT'd nodes never get a valid PublicInternet
    /// network class and cannot allocate private routes.
    ///
    /// When true: outbound-only peer, relays handle inbound. Route allocation
    /// and DHT operations work normally. Veilid also disables UPNP and
    /// detect_address_changes internally when this is set.
    #[serde(default)]
    pub require_inbound_relay: bool,

    // ── Private network ─────────────────────────────────────────────
    /// Network isolation password. Nodes with different passwords cannot
    /// communicate. Use for isolated test/staging/production environments.
    /// None / empty = public Veilid network (default).
    #[serde(default)]
    pub network_key_password: Option<String>,

    // ── Bootstrap ───────────────────────────────────────────────────
    /// Bootstrap node addresses. Empty = Veilid Foundation bootstrap.
    /// Override for private/isolated networks.
    #[serde(default)]
    pub bootstrap: Vec<String>,

    /// Bootstrap node signing keys (trust anchors). Empty = Veilid Foundation keys.
    /// Override when running a private bootstrap with custom signing keys.
    #[serde(default)]
    pub bootstrap_keys: Vec<String>,

    // ── Routing table attachment thresholds ─────────────────────────
    /// Peer count threshold for OverAttached state. Default: 64.
    #[serde(default = "default_limit_over_attached")]
    pub limit_over_attached: u32,

    /// Peer count threshold for FullyAttached state. Default: 32.
    /// Reduce for small test networks (e.g. 6 for a 3-node test harness).
    #[serde(default = "default_limit_fully_attached")]
    pub limit_fully_attached: u32,

    /// Peer count threshold for AttachedStrong state. Default: 16.
    #[serde(default = "default_limit_attached_strong")]
    pub limit_attached_strong: u32,

    /// Peer count threshold for AttachedGood state. Default: 8.
    #[serde(default = "default_limit_attached_good")]
    pub limit_attached_good: u32,

    /// Peer count threshold for AttachedWeak state. Default: 4.
    #[serde(default = "default_limit_attached_weak")]
    pub limit_attached_weak: u32,

    // ── RPC ─────────────────────────────────────────────────────────
    /// RPC concurrency limit. 0 = automatic. Default: 0.
    #[serde(default)]
    pub rpc_concurrency: u32,

    /// RPC queue depth. Default: 1024.
    #[serde(default = "default_rpc_queue_size")]
    pub rpc_queue_size: u32,

    /// RPC operation timeout in milliseconds. Default: 5000.
    #[serde(default = "default_rpc_timeout_ms_veilid")]
    pub rpc_timeout_ms: u32,

    /// Maximum accepted timestamp age behind wall-clock in ms. None = 10000.
    /// Tighten to reject replayed old messages; loosen for high-latency links.
    #[serde(default = "default_rpc_max_timestamp_behind_ms")]
    pub rpc_max_timestamp_behind_ms: Option<u32>,

    /// Maximum accepted timestamp ahead of wall-clock in ms. None = 10000.
    #[serde(default = "default_rpc_max_timestamp_ahead_ms")]
    pub rpc_max_timestamp_ahead_ms: Option<u32>,

    /// Maximum route hop count (1-5). Default: 4.
    #[serde(default = "default_rpc_max_route_hop_count")]
    pub rpc_max_route_hop_count: u8,

    /// Default route hop count. Default: 1.
    #[serde(default = "default_rpc_default_route_hop_count")]
    pub rpc_default_route_hop_count: u8,

    // ── DHT ─────────────────────────────────────────────────────────
    /// Maximum nodes contacted in a find_node fanout. Default: 20.
    #[serde(default = "default_dht_max_find_node_count")]
    pub dht_max_find_node_count: u32,

    /// Timeout for resolve_node RPC in milliseconds. Default: 10000.
    #[serde(default = "default_dht_resolve_node_timeout_ms")]
    pub dht_resolve_node_timeout_ms: u32,

    /// Number of nodes to resolve per lookup. Default: 1.
    #[serde(default = "default_dht_resolve_node_count")]
    pub dht_resolve_node_count: u32,

    /// Fanout for resolve_node queries. Default: 5.
    #[serde(default = "default_dht_resolve_node_fanout")]
    pub dht_resolve_node_fanout: u32,

    /// Timeout for get_value RPC in milliseconds. Default: 10000.
    #[serde(default = "default_dht_get_value_timeout_ms")]
    pub dht_get_value_timeout_ms: u32,

    /// Timeout for set_value RPC in milliseconds. Default: 10000.
    #[serde(default = "default_dht_set_value_timeout_ms")]
    pub dht_set_value_timeout_ms: u32,

    /// Minimum peers required for DHT operations. Default: 20.
    #[serde(default = "default_dht_min_peer_count")]
    pub dht_min_peer_count: u32,

    /// Minimum time between peer refresh cycles in ms. Default: 60000.
    #[serde(default = "default_dht_min_peer_refresh_time_ms")]
    pub dht_min_peer_refresh_time_ms: u32,

    /// Timeout for dial_info validation receipts in ms. Default: 1000.
    #[serde(default = "default_dht_validate_dial_info_receipt_time_ms")]
    pub dht_validate_dial_info_receipt_time_ms: u32,

    /// Local subkey cache size (entries). Default: 1024.
    #[serde(default = "default_dht_local_subkey_cache_size")]
    pub dht_local_subkey_cache_size: u32,

    /// Local subkey cache memory limit in MB. Default: system RAM / 32.
    /// Set explicitly to cap memory usage on memory-constrained hosts.
    #[serde(default = "default_dht_local_max_subkey_cache_memory_mb")]
    pub dht_local_max_subkey_cache_memory_mb: u32,

    /// Remote subkey cache size (entries). Default: 128.
    #[serde(default = "default_dht_remote_subkey_cache_size")]
    pub dht_remote_subkey_cache_size: u32,

    /// Maximum remote DHT records to cache. Default: 128.
    #[serde(default = "default_dht_remote_max_records")]
    pub dht_remote_max_records: u32,

    /// Maximum remote subkey cache memory in MB. Default: 256.
    #[serde(default = "default_dht_remote_max_subkey_cache_memory_mb")]
    pub dht_remote_max_subkey_cache_memory_mb: u32,

    /// Maximum DHT storage space in MB. Default: 256.
    #[serde(default = "default_dht_remote_max_storage_space_mb")]
    pub dht_remote_max_storage_space_mb: u32,

    /// DHT set value fanout. Default: 6.
    #[serde(default = "default_dht_set_value_fanout")]
    pub dht_set_value_fanout: u32,

    /// DHT get value fanout. Default: 5.
    #[serde(default = "default_dht_get_value_fanout")]
    pub dht_get_value_fanout: u32,

    /// DHT set value redundancy count. Default: 5.
    #[serde(default = "default_dht_set_value_count")]
    pub dht_set_value_count: u32,

    /// DHT get value redundancy count. Default: 3.
    #[serde(default = "default_dht_get_value_count")]
    pub dht_get_value_count: u32,

    /// DHT consensus width (must be >= set_value_count). Default: 10.
    #[serde(default = "default_dht_consensus_width")]
    pub dht_consensus_width: u32,

    /// Maximum DHT watch subscriptions served to public peers. Default: 32.
    #[serde(default = "default_dht_public_watch_limit")]
    pub dht_public_watch_limit: u32,

    /// Maximum DHT watch subscriptions served to community members. Default: 8.
    #[serde(default = "default_dht_member_watch_limit")]
    pub dht_member_watch_limit: u32,

    /// Maximum watch TTL in milliseconds. Default: 600000 (10 min).
    #[serde(default = "default_dht_max_watch_expiration_ms")]
    pub dht_max_watch_expiration_ms: u32,

    /// Maximum concurrent inbound DHT transactions for public peers. Default: 4.
    #[serde(default = "default_dht_public_transaction_limit")]
    pub dht_public_transaction_limit: u32,

    /// Maximum concurrent inbound DHT transactions for community members. Default: 1.
    #[serde(default = "default_dht_member_transaction_limit")]
    pub dht_member_transaction_limit: u32,

    // ── Protected store ─────────────────────────────────────────────
    /// Always use insecure file-based storage, even when a keyring is available.
    /// Use in CI or ephemeral environments where keyring fails. Default: false.
    #[serde(default)]
    pub always_use_insecure_storage: bool,

    /// Delete the protected store on next startup (wipes Veilid identity keys).
    /// Set true only for dev/test environment resets. Default: false.
    #[serde(default)]
    pub protected_store_delete: bool,

    /// Device encryption key password for the protected store. Default: "".
    #[serde(default)]
    pub protected_store_device_encryption_key_password: String,

    // ── Table store ─────────────────────────────────────────────────
    /// Delete the table store on next startup (wipes DHT routing table).
    /// Set true only for dev/test environment resets. Default: false.
    #[serde(default)]
    pub table_store_delete: bool,

    // ── Block store ─────────────────────────────────────────────────
    /// Delete the block store on next startup. Default: false.
    #[serde(default)]
    pub block_store_delete: bool,

    // ── Capabilities ────────────────────────────────────────────────
    /// Veilid capabilities to disable. Empty = all capabilities enabled (default).
    /// See veilid_core::VeilidCapability for valid capability name strings.
    #[serde(default)]
    pub disable_capabilities: Vec<String>,
}

impl Default for VeilidNetworkConfig {
    fn default() -> Self {
        Self {
            // Protocol: TCP
            tcp_listen_address: String::new(),
            tcp_public_address: None,
            tcp_listen: true,
            tcp_connect: true,
            tcp_max_connections: default_tcp_max_connections(),
            // Protocol: UDP
            udp_listen_address: String::new(),
            udp_public_address: None,
            udp_enabled: true,
            udp_socket_pool_size: 0,
            // Protocol: WebSocket
            ws_listen_address: String::new(),
            ws_listen: true,
            ws_connect: true,
            ws_max_connections: default_ws_max_connections(),
            ws_path: default_ws_path(),
            // Connection limits
            max_connections_per_ip4: default_max_connections_per_ip4(),
            max_connections_per_ip6_prefix: default_max_connections_per_ip6_prefix(),
            max_connections_per_ip6_prefix_size: default_max_connections_per_ip6_prefix_size(),
            max_connection_frequency_per_min: default_max_connection_frequency_per_min(),
            client_allowlist_timeout_ms: default_client_allowlist_timeout_ms(),
            reverse_connection_receipt_time_ms: default_reverse_connection_receipt_time_ms(),
            hole_punch_receipt_time_ms: default_hole_punch_receipt_time_ms(),
            // Connection timeouts
            connection_initial_timeout_ms: default_connection_initial_timeout_ms(),
            connection_inactivity_timeout_ms: default_connection_inactivity_timeout_ms(),
            // NAT / address detection
            upnp: true,
            detect_address_changes: default_detect_address_changes(),
            restricted_nat_retries: 0,
            require_inbound_relay: false,
            // Private network
            network_key_password: None,
            // Bootstrap
            bootstrap: Vec::new(),
            bootstrap_keys: Vec::new(),
            // Routing table thresholds
            limit_over_attached: default_limit_over_attached(),
            limit_fully_attached: default_limit_fully_attached(),
            limit_attached_strong: default_limit_attached_strong(),
            limit_attached_good: default_limit_attached_good(),
            limit_attached_weak: default_limit_attached_weak(),
            // RPC
            rpc_concurrency: 0,
            rpc_queue_size: default_rpc_queue_size(),
            rpc_timeout_ms: default_rpc_timeout_ms_veilid(),
            rpc_max_timestamp_behind_ms: default_rpc_max_timestamp_behind_ms(),
            rpc_max_timestamp_ahead_ms: default_rpc_max_timestamp_ahead_ms(),
            rpc_max_route_hop_count: default_rpc_max_route_hop_count(),
            rpc_default_route_hop_count: default_rpc_default_route_hop_count(),
            // DHT
            dht_max_find_node_count: default_dht_max_find_node_count(),
            dht_resolve_node_timeout_ms: default_dht_resolve_node_timeout_ms(),
            dht_resolve_node_count: default_dht_resolve_node_count(),
            dht_resolve_node_fanout: default_dht_resolve_node_fanout(),
            dht_get_value_timeout_ms: default_dht_get_value_timeout_ms(),
            dht_set_value_timeout_ms: default_dht_set_value_timeout_ms(),
            dht_min_peer_count: default_dht_min_peer_count(),
            dht_min_peer_refresh_time_ms: default_dht_min_peer_refresh_time_ms(),
            dht_validate_dial_info_receipt_time_ms: default_dht_validate_dial_info_receipt_time_ms(),
            dht_local_subkey_cache_size: default_dht_local_subkey_cache_size(),
            dht_local_max_subkey_cache_memory_mb: default_dht_local_max_subkey_cache_memory_mb(),
            dht_remote_subkey_cache_size: default_dht_remote_subkey_cache_size(),
            dht_remote_max_records: default_dht_remote_max_records(),
            dht_remote_max_subkey_cache_memory_mb: default_dht_remote_max_subkey_cache_memory_mb(),
            dht_remote_max_storage_space_mb: default_dht_remote_max_storage_space_mb(),
            dht_set_value_fanout: default_dht_set_value_fanout(),
            dht_get_value_fanout: default_dht_get_value_fanout(),
            dht_set_value_count: default_dht_set_value_count(),
            dht_get_value_count: default_dht_get_value_count(),
            dht_consensus_width: default_dht_consensus_width(),
            dht_public_watch_limit: default_dht_public_watch_limit(),
            dht_member_watch_limit: default_dht_member_watch_limit(),
            dht_max_watch_expiration_ms: default_dht_max_watch_expiration_ms(),
            dht_public_transaction_limit: default_dht_public_transaction_limit(),
            dht_member_transaction_limit: default_dht_member_transaction_limit(),
            // Protected store
            always_use_insecure_storage: false,
            protected_store_delete: false,
            protected_store_device_encryption_key_password: String::new(),
            // Table store
            table_store_delete: false,
            // Block store
            block_store_delete: false,
            // Capabilities
            disable_capabilities: Vec::new(),
        }
    }
}

// ── Per-data-class safety routing ──────────────────────────────────────────

/// Per-data-class safety routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyConfig {
    /// Safety profile for text messages (DM + community gossip).
    #[serde(default = "SafetyProfile::default_text")]
    pub text: SafetyProfile,

    /// Safety profile for voice packets.
    #[serde(default = "SafetyProfile::default_voice")]
    pub voice: SafetyProfile,

    /// Safety profile for DHT record operations.
    #[serde(default = "SafetyProfile::default_dht")]
    pub dht: SafetyProfile,

    /// Safety profile for RPC calls (bootstrap, MEK transfer, sync).
    #[serde(default = "SafetyProfile::default_rpc")]
    pub rpc: SafetyProfile,
}

/// Privacy/performance parameters for a single data class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyProfile {
    /// Extra hops for sender privacy. 0 = no safety route (direct).
    /// 1 = one relay hop (default).
    pub hop_count: u8,

    /// Prefer connection reliability or low latency.
    #[serde(default)]
    pub stability: StabilityPreference,

    /// Message ordering preference.
    #[serde(default)]
    pub sequencing: SequencingPreference,
}

/// Stability preference -- maps to Veilid's `Stability` enum internally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StabilityPreference {
    /// Prefer low latency over reliability (may drop packets).
    LowLatency,
    /// Prefer reliable delivery (default).
    #[default]
    Reliable,
}

/// Sequencing preference -- maps to Veilid's `Sequencing` enum internally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SequencingPreference {
    /// No ordering guarantee.
    NoPreference,
    /// Prefer ordered delivery (default).
    #[default]
    PreferOrdered,
    /// Require strict ordering (may fail if not achievable).
    EnsureOrdered,
}

impl SafetyProfile {
    pub fn default_text() -> Self {
        Self {
            hop_count: 1,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
        }
    }

    pub fn default_voice() -> Self {
        Self {
            hop_count: 1,
            stability: StabilityPreference::LowLatency,
            sequencing: SequencingPreference::NoPreference,
        }
    }

    pub fn default_dht() -> Self {
        Self {
            hop_count: 1,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
        }
    }

    pub fn default_rpc() -> Self {
        Self {
            hop_count: 1,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::EnsureOrdered,
        }
    }
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            text: SafetyProfile::default_text(),
            voice: SafetyProfile::default_voice(),
            dht: SafetyProfile::default_dht(),
            rpc: SafetyProfile::default_rpc(),
        }
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            storage_dir: "~/.rekindle".into(),
            namespace: default_namespace(),
            safety: SafetyConfig::default(),
            rpc_timeout_ms: default_rpc_timeout_ms(),
            dht_write_retries: default_dht_write_retries(),
            route_refresh_secs: default_route_refresh_secs(),
            route_cache_ttl_secs: default_route_cache_ttl_secs(),
            circuit_breaker_threshold: default_circuit_breaker_threshold(),
            circuit_breaker_cooldown_secs: default_circuit_breaker_cooldown_secs(),
            dedup_cache_capacity: default_dedup_cache_capacity(),
            gossip_ttl: default_gossip_ttl(),
            allow_insecure_protected_store: false,
            metrics_port: default_metrics_port(),
            health_port: default_health_port(),
            veilid: VeilidNetworkConfig::default(),
        }
    }
}

// ── Default value functions ─────────────────────────────────────────────────

fn default_namespace() -> String { "rekindle".into() }
fn default_rpc_timeout_ms() -> u64 { 8_000 }
fn default_dht_write_retries() -> u32 { 3 }
fn default_route_refresh_secs() -> u64 { 60 }
fn default_route_cache_ttl_secs() -> u64 { 90 }
fn default_circuit_breaker_threshold() -> u32 { 3 }
fn default_circuit_breaker_cooldown_secs() -> u64 { 45 }
fn default_dedup_cache_capacity() -> usize { 2048 }
fn default_gossip_ttl() -> u8 { 5 }
fn default_metrics_port() -> u16 { 9191 }
fn default_health_port() -> u16 { 9192 }

fn default_true() -> bool { true }
fn default_tcp_max_connections() -> u32 { 32 }
fn default_ws_max_connections() -> u32 { 32 }
fn default_ws_path() -> String { "ws".into() }
fn default_max_connections_per_ip4() -> u32 { 32 }
fn default_max_connections_per_ip6_prefix() -> u32 { 32 }
fn default_max_connections_per_ip6_prefix_size() -> u32 { 56 }
fn default_max_connection_frequency_per_min() -> u32 { 128 }
fn default_connection_initial_timeout_ms() -> u32 { 2000 }
fn default_connection_inactivity_timeout_ms() -> u32 { 60_000 }
fn default_limit_over_attached() -> u32 { 64 }
fn default_limit_fully_attached() -> u32 { 32 }
fn default_limit_attached_strong() -> u32 { 16 }
fn default_limit_attached_good() -> u32 { 8 }
fn default_limit_attached_weak() -> u32 { 4 }
fn default_rpc_queue_size() -> u32 { 1024 }
fn default_rpc_timeout_ms_veilid() -> u32 { 5000 }
fn default_rpc_max_route_hop_count() -> u8 { 4 }
fn default_rpc_default_route_hop_count() -> u8 { 1 }
fn default_dht_public_watch_limit() -> u32 { 32 }
fn default_dht_member_watch_limit() -> u32 { 8 }
fn default_dht_max_watch_expiration_ms() -> u32 { 600_000 }
fn default_dht_remote_max_records() -> u32 { 128 }
fn default_dht_remote_max_subkey_cache_memory_mb() -> u32 { 256 }
fn default_dht_remote_max_storage_space_mb() -> u32 { 256 }
fn default_dht_set_value_fanout() -> u32 { 6 }
fn default_dht_get_value_fanout() -> u32 { 5 }
fn default_dht_set_value_count() -> u32 { 5 }
fn default_dht_get_value_count() -> u32 { 3 }
fn default_dht_consensus_width() -> u32 { 10 }
fn default_client_allowlist_timeout_ms() -> u32 { 300_000 }
fn default_reverse_connection_receipt_time_ms() -> u32 { 5_000 }
fn default_hole_punch_receipt_time_ms() -> u32 { 5_000 }
fn default_detect_address_changes() -> Option<bool> { None }
fn default_rpc_max_timestamp_behind_ms() -> Option<u32> { None }
fn default_rpc_max_timestamp_ahead_ms() -> Option<u32> { None }
fn default_dht_max_find_node_count() -> u32 { 20 }
fn default_dht_resolve_node_timeout_ms() -> u32 { 10_000 }
fn default_dht_resolve_node_count() -> u32 { 1 }
fn default_dht_resolve_node_fanout() -> u32 { 5 }
fn default_dht_get_value_timeout_ms() -> u32 { 10_000 }
fn default_dht_set_value_timeout_ms() -> u32 { 10_000 }
fn default_dht_min_peer_count() -> u32 { 20 }
fn default_dht_min_peer_refresh_time_ms() -> u32 { 60_000 }
fn default_dht_validate_dial_info_receipt_time_ms() -> u32 { 1_000 }
fn default_dht_local_subkey_cache_size() -> u32 { 1024 }
fn default_dht_local_max_subkey_cache_memory_mb() -> u32 { 256 }
fn default_dht_remote_subkey_cache_size() -> u32 { 128 }
fn default_dht_public_transaction_limit() -> u32 { 4 }
fn default_dht_member_transaction_limit() -> u32 { 1 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_config_default_round_trip() {
        let cfg = TransportConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: TransportConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.namespace, "rekindle");
        assert_eq!(parsed.rpc_timeout_ms, 8_000);
        assert_eq!(parsed.metrics_port, 9191);
        assert_eq!(parsed.health_port, 9192);
    }

    #[test]
    fn veilid_network_config_defaults_match_veilid() {
        let cfg = VeilidNetworkConfig::default();
        assert!(cfg.tcp_listen);
        assert!(cfg.tcp_connect);
        assert!(cfg.udp_enabled);
        assert!(cfg.ws_listen);
        assert!(cfg.ws_connect);
        assert_eq!(cfg.tcp_max_connections, 32);
        assert_eq!(cfg.max_connections_per_ip4, 32);
        assert_eq!(cfg.limit_fully_attached, 32);
        assert_eq!(cfg.limit_attached_good, 8);
        assert_eq!(cfg.rpc_max_route_hop_count, 4);
        assert_eq!(cfg.dht_public_watch_limit, 32);
        assert!(cfg.upnp);
        assert!(cfg.detect_address_changes.is_none()); // None = auto
        assert!(cfg.network_key_password.is_none());
        assert!(cfg.bootstrap.is_empty());
        assert!(cfg.disable_capabilities.is_empty());
    }

    #[test]
    fn safety_profile_defaults() {
        let text = SafetyProfile::default_text();
        assert_eq!(text.hop_count, 1);
        assert_eq!(text.stability, StabilityPreference::Reliable);

        let voice = SafetyProfile::default_voice();
        assert_eq!(voice.stability, StabilityPreference::LowLatency);
        assert_eq!(voice.sequencing, SequencingPreference::NoPreference);
    }

    #[test]
    fn veilid_config_toml_round_trip() {
        let cfg = TransportConfig::default();
        let toml = toml::to_string(&cfg).unwrap();
        let parsed: TransportConfig = toml::from_str(&toml).unwrap();
        assert_eq!(parsed.veilid.tcp_max_connections, 32);
        assert_eq!(parsed.veilid.limit_fully_attached, 32);
        assert_eq!(parsed.veilid.dht_public_watch_limit, 32);
    }
}
