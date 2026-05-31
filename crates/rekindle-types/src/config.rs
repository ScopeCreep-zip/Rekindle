//! Transport configuration types for the IPC boundary.
//!
//! These structs are constructed by CLI config loading and passed to the
//! node daemon at startup. Defined here in `rekindle-types` so that both
//! the CLI (config producer) and the node/transport (config consumer) can
//! use them without the CLI depending on `rekindle-transport`.

use serde::{Deserialize, Serialize};

/// Top-level transport configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
}

/// Per-data-class safety routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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

    /// **Phase 9** — whether to wrap the sender in a Veilid safety route
    /// (anonymous) or send directly from the personal private route
    /// (identified). Independent of `hop_count`: safety routes still
    /// need hops, direct routes never do.
    ///
    /// `true` for most user-facing actions (DM text, DHT writes, RPC) —
    /// even a 1-hop safety route hides "who sent this" from the
    /// destination's incoming relay.
    ///
    /// `false` for voice calls (the recipient already knows we're
    /// calling them; latency is paramount) and other contexts where
    /// the user has explicitly opted out of anonymity.
    #[serde(default = "default_true")]
    pub sender_anonymous: bool,
}

fn default_true() -> bool { true }

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
            sender_anonymous: true,
        }
    }

    pub fn default_voice() -> Self {
        Self {
            hop_count: 1,
            stability: StabilityPreference::LowLatency,
            sequencing: SequencingPreference::NoPreference,
            // Voice: latency over anonymity. Sender is direct.
            sender_anonymous: false,
        }
    }

    pub fn default_dht() -> Self {
        Self {
            hop_count: 1,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
            sender_anonymous: true,
        }
    }

    pub fn default_rpc() -> Self {
        Self {
            hop_count: 1,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::EnsureOrdered,
            sender_anonymous: true,
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
        }
    }
}

fn default_namespace() -> String { "rekindle".into() }
fn default_rpc_timeout_ms() -> u64 { 8_000 }
fn default_dht_write_retries() -> u32 { 3 }
fn default_route_refresh_secs() -> u64 { 60 }
fn default_route_cache_ttl_secs() -> u64 { 90 }
fn default_circuit_breaker_threshold() -> u32 { 3 }
fn default_circuit_breaker_cooldown_secs() -> u64 { 45 }
fn default_dedup_cache_capacity() -> usize { 2048 }
fn default_gossip_ttl() -> u8 { 5 }

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
}
