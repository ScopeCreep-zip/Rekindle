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

    /// Veilid network configuration. Controls Veilid-level tunables:
    /// protocol ports, DHT cache sizing, UPNP, and more.
    ///
    /// Many per-connection and per-IP fields were removed in veilid-core
    /// 0.5.7 and are now managed internally. This struct exposes only
    /// the fields that the public VeilidConfig API still accepts.
    #[serde(default)]
    pub veilid: VeilidNetworkConfig,
}

// -- Veilid network configuration (veilid-core 0.5.7+) ---------------------

/// Veilid network configuration.
///
/// Exposes the fields that veilid-core 0.5.7 still accepts via its public
/// `VeilidConfig` API. Per-connection limits, routing-table attachment
/// thresholds, RPC queue/timeout tuning, and DHT fanout/consensus
/// parameters were removed in 0.5.7 and are now managed internally
/// by veilid-core with sensible defaults.
///
/// Mapped onto `veilid_core::VeilidConfig` in `TransportNode::start()`
/// before `api_startup` is called.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VeilidNetworkConfig {
    // -- Protocol: TCP ------------------------------------------------------
    /// TCP listen address. Empty = Veilid default (port 5150, search upward).
    #[serde(default)]
    pub tcp_listen_address: String,

    /// Static public TCP address advertised to peers. None = auto-detect.
    #[serde(default)]
    pub tcp_public_address: Option<String>,

    /// Enable inbound TCP connections. Default: true.
    #[serde(default = "default_true")]
    pub tcp_listen: bool,

    /// Enable outbound TCP connections. Default: true.
    #[serde(default = "default_true")]
    pub tcp_connect: bool,

    // -- Protocol: UDP ------------------------------------------------------
    /// UDP listen address. Empty = Veilid default (port 5150, search upward).
    #[serde(default)]
    pub udp_listen_address: String,

    /// Static public UDP address advertised to peers. None = auto-detect.
    #[serde(default)]
    pub udp_public_address: Option<String>,

    /// Enable UDP transport. Default: true.
    #[serde(default = "default_true")]
    pub udp_enabled: bool,

    // -- Protocol: WebSocket ------------------------------------------------
    /// WebSocket listen address. Empty = Veilid default (port 5150).
    #[serde(default)]
    pub ws_listen_address: String,

    /// Enable inbound WebSocket connections. Default: true.
    #[serde(default = "default_true")]
    pub ws_listen: bool,

    /// Enable outbound WebSocket connections. Default: true.
    #[serde(default = "default_true")]
    pub ws_connect: bool,

    /// WebSocket URL path. Default: "ws".
    #[serde(default = "default_ws_path")]
    pub ws_path: String,

    // -- NAT / address detection --------------------------------------------
    /// Enable UPnP for automatic port forwarding. Default: true.
    #[serde(default = "default_true")]
    pub upnp: bool,

    /// Monitor and react to network interface address changes. Default: true.
    /// `None` means auto: Veilid enables this when no globally-routable
    /// interface address is found.
    #[serde(default = "default_detect_address_changes")]
    pub detect_address_changes: Option<bool>,

    /// Require inbound relay for all inbound connections. Default: false.
    ///
    /// Set true when the node is behind NAT with no direct inbound reachability
    /// (Docker bridge, cloud NAT, CGNAT). Veilid will skip dial-info validation
    /// and use relays instead.
    #[serde(default)]
    pub require_inbound_relay: bool,

    // -- Private network ----------------------------------------------------
    /// Network isolation password. Nodes with different passwords cannot
    /// communicate. None / empty = public Veilid network (default).
    #[serde(default)]
    pub network_key_password: Option<String>,

    // -- Bootstrap ----------------------------------------------------------
    /// Bootstrap node addresses. Empty = Veilid Foundation bootstrap.
    #[serde(default)]
    pub bootstrap: Vec<String>,

    /// Bootstrap node signing keys. Empty = Veilid Foundation keys.
    #[serde(default)]
    pub bootstrap_keys: Vec<String>,

    // -- RPC ----------------------------------------------------------------
    /// Default route hop count. Default: 1.
    #[serde(default = "default_rpc_default_route_hop_count")]
    pub rpc_default_route_hop_count: u8,

    // -- DHT (cache/storage sizing) -----------------------------------------
    /// Local subkey cache size (entries). Default: 1024.
    #[serde(default = "default_dht_local_subkey_cache_size")]
    pub dht_local_subkey_cache_size: u32,

    /// Local subkey cache memory limit in MB. Default: 256.
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

    // -- Protected store ----------------------------------------------------
    /// Always use insecure file-based storage, even when a keyring is available.
    #[serde(default)]
    pub always_use_insecure_storage: bool,

    /// Delete the protected store on next startup. Default: false.
    #[serde(default)]
    pub protected_store_delete: bool,

    /// Device encryption key password for the protected store. Default: "".
    #[serde(default)]
    pub protected_store_device_encryption_key_password: String,

    // -- Table store --------------------------------------------------------
    /// Delete the table store on next startup. Default: false.
    #[serde(default)]
    pub table_store_delete: bool,

    // -- Block store --------------------------------------------------------
    /// Delete the block store on next startup. Default: false.
    #[serde(default)]
    pub block_store_delete: bool,

    // -- Capabilities -------------------------------------------------------
    /// Veilid capabilities to disable. Empty = all capabilities enabled.
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
            // Protocol: UDP
            udp_listen_address: String::new(),
            udp_public_address: None,
            udp_enabled: true,
            // Protocol: WebSocket
            ws_listen_address: String::new(),
            ws_listen: true,
            ws_connect: true,
            ws_path: default_ws_path(),
            // NAT / address detection
            upnp: true,
            detect_address_changes: default_detect_address_changes(),
            require_inbound_relay: false,
            // Private network
            network_key_password: None,
            // Bootstrap
            bootstrap: Vec::new(),
            bootstrap_keys: Vec::new(),
            // RPC
            rpc_default_route_hop_count: default_rpc_default_route_hop_count(),
            // DHT
            dht_local_subkey_cache_size: default_dht_local_subkey_cache_size(),
            dht_local_max_subkey_cache_memory_mb: default_dht_local_max_subkey_cache_memory_mb(),
            dht_remote_subkey_cache_size: default_dht_remote_subkey_cache_size(),
            dht_remote_max_records: default_dht_remote_max_records(),
            dht_remote_max_subkey_cache_memory_mb: default_dht_remote_max_subkey_cache_memory_mb(),
            dht_remote_max_storage_space_mb: default_dht_remote_max_storage_space_mb(),
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

// -- Per-data-class safety routing ------------------------------------------

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

// -- Default value functions ------------------------------------------------

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
fn default_ws_path() -> String { "ws".into() }
fn default_detect_address_changes() -> Option<bool> { None }
fn default_rpc_default_route_hop_count() -> u8 { 1 }
fn default_dht_local_subkey_cache_size() -> u32 { 1024 }
fn default_dht_local_max_subkey_cache_memory_mb() -> u32 { 256 }
fn default_dht_remote_subkey_cache_size() -> u32 { 128 }
fn default_dht_remote_max_records() -> u32 { 128 }
fn default_dht_remote_max_subkey_cache_memory_mb() -> u32 { 256 }
fn default_dht_remote_max_storage_space_mb() -> u32 { 256 }

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
    fn veilid_network_config_defaults() {
        let cfg = VeilidNetworkConfig::default();
        assert!(cfg.tcp_listen);
        assert!(cfg.tcp_connect);
        assert!(cfg.udp_enabled);
        assert!(cfg.ws_listen);
        assert!(cfg.ws_connect);
        assert!(cfg.upnp);
        assert!(cfg.detect_address_changes.is_none());
        assert!(cfg.network_key_password.is_none());
        assert!(cfg.bootstrap.is_empty());
        assert!(cfg.disable_capabilities.is_empty());
        assert_eq!(cfg.rpc_default_route_hop_count, 1);
        assert_eq!(cfg.dht_local_subkey_cache_size, 1024);
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
        assert_eq!(parsed.veilid.rpc_default_route_hop_count, 1);
        assert_eq!(parsed.veilid.dht_local_subkey_cache_size, 1024);
    }
}
