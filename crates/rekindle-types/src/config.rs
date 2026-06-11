//! Transport configuration types for the IPC boundary.
//!
//! These structs are constructed by CLI config loading and passed to the
//! node daemon at startup. Defined here in `rekindle-types` so that both
//! the CLI (config producer) and the node/transport (config consumer) can
//! use them without the CLI depending on `rekindle-transport`.

use serde::{Deserialize, Serialize};

/// Veilid Safe-route relay hops applied to **every** path, voice and video included.
///
/// 3 = "Tor-class": a middle relay means no guard+exit collusion of two nodes can
/// link sender to receiver. This is the single anonymity floor — `hop_count` is never
/// lowered per data class; only `stability`/`sequencing` vary. veilid-core also rejects
/// a Safe route with `hop_count == 0`, so the floor is always a valid route.
pub const ANONYMITY_HOP_FLOOR: u8 = 3;

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

    /// Cadence in seconds of the attached-but-routeless watchdog.
    /// Routes are event-driven (healed on `RouteChange` death), never
    /// rotated on a timer — this only backstops missed heals.
    #[serde(default = "default_route_watchdog_secs", alias = "routeRefreshSecs")]
    pub route_watchdog_secs: u64,

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
///
/// Every class routes through a Veilid **safety route** (sender hidden
/// behind an ephemeral route id) at the uniform [`ANONYMITY_HOP_FLOOR`].
/// Classes differ only in `stability`/`sequencing` (latency/ordering
/// knobs) — never in anonymity. Voice keeps `LowLatency` stability, the
/// lowest-latency variant *within* the 3-hop Safe floor, rather than
/// trading anonymity for speed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyProfile {
    /// Safety-route relay hops. Floor-enforced to [`ANONYMITY_HOP_FLOOR`]
    /// by the transport builder; values below it are clamped up.
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
            hop_count: ANONYMITY_HOP_FLOOR,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
        }
    }

    pub fn default_voice() -> Self {
        Self {
            // Voice routes through the same 3-hop Tor-class floor as every
            // other class; LowLatency stability is the lowest-latency
            // variant *within* that anonymous floor (not an Unsafe route).
            hop_count: ANONYMITY_HOP_FLOOR,
            stability: StabilityPreference::LowLatency,
            sequencing: SequencingPreference::NoPreference,
        }
    }

    pub fn default_dht() -> Self {
        Self {
            hop_count: ANONYMITY_HOP_FLOOR,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
        }
    }

    pub fn default_rpc() -> Self {
        Self {
            hop_count: ANONYMITY_HOP_FLOOR,
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
            route_watchdog_secs: default_route_watchdog_secs(),
            route_cache_ttl_secs: default_route_cache_ttl_secs(),
            circuit_breaker_threshold: default_circuit_breaker_threshold(),
            circuit_breaker_cooldown_secs: default_circuit_breaker_cooldown_secs(),
            dedup_cache_capacity: default_dedup_cache_capacity(),
            gossip_ttl: default_gossip_ttl(),
            allow_insecure_protected_store: false,
        }
    }
}

fn default_namespace() -> String {
    "rekindle".into()
}
fn default_rpc_timeout_ms() -> u64 {
    8_000
}
fn default_dht_write_retries() -> u32 {
    3
}
fn default_route_watchdog_secs() -> u64 {
    30
}
fn default_route_cache_ttl_secs() -> u64 {
    90
}
fn default_circuit_breaker_threshold() -> u32 {
    3
}
fn default_circuit_breaker_cooldown_secs() -> u64 {
    45
}
fn default_dedup_cache_capacity() -> usize {
    2048
}
fn default_gossip_ttl() -> u8 {
    5
}

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
        assert_eq!(parsed.route_watchdog_secs, 30);
    }

    #[test]
    fn old_route_refresh_key_still_parses() {
        // Pre-rename configs carry `routeRefreshSecs` (camelCase wire);
        // the serde alias must map it onto the watchdog cadence.
        let json = r#"{"storageDir": "/tmp/x", "routeRefreshSecs": 45}"#;
        let parsed: TransportConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.route_watchdog_secs, 45);
    }

    #[test]
    fn safety_profile_defaults() {
        let text = SafetyProfile::default_text();
        assert_eq!(text.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(text.stability, StabilityPreference::Reliable);

        // Voice rides the same anonymity floor as text; only its
        // stability/sequencing differ (lowest-latency *within* the floor).
        let voice = SafetyProfile::default_voice();
        assert_eq!(voice.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(voice.stability, StabilityPreference::LowLatency);
        assert_eq!(voice.sequencing, SequencingPreference::NoPreference);
    }

    #[test]
    fn every_class_default_is_at_or_above_the_floor() {
        for p in [
            SafetyProfile::default_text(),
            SafetyProfile::default_voice(),
            SafetyProfile::default_dht(),
            SafetyProfile::default_rpc(),
        ] {
            assert!(
                p.hop_count >= ANONYMITY_HOP_FLOOR,
                "no data class may route below the anonymity floor",
            );
        }
    }
}
