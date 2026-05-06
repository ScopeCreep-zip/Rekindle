//! Configuration schema — all config structs with serde and defaults.
//!
//! Single source of truth for the config file format. Every field has a
//! documented default value. `deny_unknown_fields` catches typos at parse
//! time.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Top-level config ────────────────────────────────────────────────────

/// Root configuration struct.
///
/// Loaded from TOML files with 8-layer precedence. See `loader.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Config schema version for forward compatibility.
    #[serde(default = "default_config_version")]
    #[allow(clippy::struct_field_names)]
    pub config_version: u32,

    /// Global settings (storage, namespace).
    #[serde(default)]
    pub global: GlobalConfig,

    /// Network and transport settings.
    #[serde(default)]
    pub network: NetworkConfig,

    /// TUI display settings.
    #[serde(default)]
    pub tui: TuiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: default_config_version(),
            global: GlobalConfig::default(),
            network: NetworkConfig::default(),
            tui: TuiConfig::default(),
        }
    }
}

// ── Global config ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    /// Veilid namespace for this application.
    #[serde(default = "default_namespace")]
    pub namespace: String,

    /// Default community to select on startup.
    #[serde(default)]
    pub default_community: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            namespace: default_namespace(),
            default_community: None,
        }
    }
}

// ── Network config ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// Per-data-class safety routing parameters.
    #[serde(default)]
    pub safety: SafetyUserConfig,

    /// RPC timeout in milliseconds.
    #[serde(default = "default_rpc_timeout_ms")]
    pub rpc_timeout_ms: u64,

    /// Maximum DHT write retries.
    #[serde(default = "default_dht_write_retries")]
    pub dht_write_retries: u32,

    /// Route refresh interval in seconds.
    #[serde(default = "default_route_refresh_secs")]
    pub route_refresh_secs: u64,

    /// Route cache TTL in seconds.
    #[serde(default = "default_route_cache_ttl_secs")]
    pub route_cache_ttl_secs: u64,

    /// Circuit breaker threshold (consecutive failures).
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,

    /// Circuit breaker cooldown in seconds.
    #[serde(default = "default_circuit_breaker_cooldown_secs")]
    pub circuit_breaker_cooldown_secs: u64,

    /// Gossip dedup cache capacity.
    #[serde(default = "default_dedup_cache_capacity")]
    pub dedup_cache_capacity: usize,

    /// Gossip TTL (max forwarding hops).
    #[serde(default = "default_gossip_ttl")]
    pub gossip_ttl: u8,

    /// Allow Veilid to fall back to encrypted file-based key storage when
    /// the OS Secret Service (dbus org.freedesktop.secrets) is unavailable.
    ///
    /// Default: false. Set to true in containers, headless servers, or
    /// environments without gnome-keyring/kwallet.
    #[serde(default)]
    pub allow_insecure_protected_store: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            safety: SafetyUserConfig::default(),
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

// ── Safety routing config ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyUserConfig {
    /// Safety profile for text messages.
    #[serde(default = "SafetyProfileUser::default_text")]
    pub text: SafetyProfileUser,

    /// Safety profile for voice packets.
    #[serde(default = "SafetyProfileUser::default_voice")]
    pub voice: SafetyProfileUser,

    /// Safety profile for DHT operations.
    #[serde(default = "SafetyProfileUser::default_dht")]
    pub dht: SafetyProfileUser,

    /// Safety profile for RPC calls.
    #[serde(default = "SafetyProfileUser::default_rpc")]
    pub rpc: SafetyProfileUser,
}

impl Default for SafetyUserConfig {
    fn default() -> Self {
        Self {
            text: SafetyProfileUser::default_text(),
            voice: SafetyProfileUser::default_voice(),
            dht: SafetyProfileUser::default_dht(),
            rpc: SafetyProfileUser::default_rpc(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyProfileUser {
    /// Extra hops for sender privacy. 0=direct, 1=default.
    #[serde(default = "default_hop_count")]
    pub hop_count: u8,

    /// Stability preference: "low_latency" or "reliable".
    #[serde(default = "default_stability")]
    pub stability: String,

    /// Sequencing preference: "no_preference", "prefer_ordered", or "ensure_ordered".
    #[serde(default = "default_sequencing")]
    pub sequencing: String,
}

impl SafetyProfileUser {
    fn default_text() -> Self {
        Self {
            hop_count: 1,
            stability: "reliable".into(),
            sequencing: "prefer_ordered".into(),
        }
    }

    fn default_voice() -> Self {
        Self {
            hop_count: 1,
            stability: "low_latency".into(),
            sequencing: "no_preference".into(),
        }
    }

    fn default_dht() -> Self {
        Self {
            hop_count: 1,
            stability: "reliable".into(),
            sequencing: "prefer_ordered".into(),
        }
    }

    fn default_rpc() -> Self {
        Self {
            hop_count: 1,
            stability: "reliable".into(),
            sequencing: "ensure_ordered".into(),
        }
    }

}

// ── TUI config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    /// Theme name (must match an opaline builtin or user theme).
    #[serde(default = "default_theme")]
    pub theme: String,

    /// Enable mouse support.
    #[serde(default = "default_true")]
    pub mouse: bool,

    /// Enable animations (skeleton loading, tachyonfx effects).
    #[serde(default = "default_true")]
    pub animations: bool,

    /// Custom keybinding overrides.
    #[serde(default)]
    pub keybindings: HashMap<String, String>,

    /// Tick rate in Hz (state updates).
    #[serde(default = "default_tick_rate")]
    pub tick_rate: f64,

    /// Frame rate in Hz (rendering).
    #[serde(default = "default_frame_rate")]
    pub frame_rate: f64,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            mouse: true,
            animations: true,
            keybindings: HashMap::new(),
            tick_rate: default_tick_rate(),
            frame_rate: default_frame_rate(),
        }
    }
}

// ── Policy config (loaded separately from /etc/rekindle/policy.toml) ────

/// Admin policy constraints that cannot be overridden by user config.
///
/// Policy fields are additive: they set minimums/maximums, they never
/// disable features that users enabled. Violations produce hard errors
/// with a message pointing to the admin.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Minimum allowed hop_count for any safety profile.
    pub min_hop_count: Option<u8>,

    /// Whether signature verification can be disabled.
    /// If true, users cannot set verify_signatures=false.
    #[serde(default)]
    pub require_signature_verification: bool,

    /// Maximum allowed gossip TTL.
    pub max_gossip_ttl: Option<u8>,
}

// ── Default value functions ─────────────────────────────────────────────

fn default_config_version() -> u32 { 1 }
fn default_namespace() -> String { "rekindle".into() }
fn default_theme() -> String { "catppuccin-latte".into() }
fn default_true() -> bool { true }
fn default_tick_rate() -> f64 { 4.0 }
fn default_frame_rate() -> f64 { 30.0 }
fn default_rpc_timeout_ms() -> u64 { 8_000 }
fn default_dht_write_retries() -> u32 { 3 }
fn default_route_refresh_secs() -> u64 { 60 }
fn default_route_cache_ttl_secs() -> u64 { 90 }
fn default_circuit_breaker_threshold() -> u32 { 3 }
fn default_circuit_breaker_cooldown_secs() -> u64 { 45 }
fn default_dedup_cache_capacity() -> usize { 2048 }
fn default_gossip_ttl() -> u8 { 5 }
fn default_hop_count() -> u8 { 1 }
fn default_stability() -> String { "reliable".into() }
fn default_sequencing() -> String { "prefer_ordered".into() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = Config::default();
        assert_eq!(cfg.config_version, 1);
        assert_eq!(cfg.global.namespace, "rekindle");
        assert_eq!(cfg.tui.theme, "catppuccin-latte");
        assert_eq!(cfg.network.rpc_timeout_ms, 8000);
        assert!(cfg.tui.mouse);
        assert!(cfg.tui.animations);
    }

    #[test]
    fn config_round_trip_toml() {
        let cfg = Config::default();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.config_version, cfg.config_version);
        assert_eq!(parsed.global.namespace, cfg.global.namespace);
    }

    #[test]
    fn deny_unknown_fields() {
        let bad_toml = r#"
            config_version = 1
            unknown_field = "hello"
        "#;
        let result: Result<Config, _> = toml::from_str(bad_toml);
        assert!(result.is_err());
    }

}
