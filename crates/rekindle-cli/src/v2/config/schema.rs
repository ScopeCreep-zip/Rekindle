//! Configuration schema — all config structs with serde and defaults.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_config_version")]
    #[allow(clippy::struct_field_names)]
    pub config_version: u32,
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub network: NetworkConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub default_community: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self { namespace: default_namespace(), default_community: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    #[serde(default)]
    pub safety: SafetyUserConfig,
    #[serde(default = "default_rpc_timeout_ms")]
    pub rpc_timeout_ms: u64,
    #[serde(default = "default_dht_write_retries")]
    pub dht_write_retries: u32,
    #[serde(default = "default_route_refresh_secs")]
    pub route_refresh_secs: u64,
    #[serde(default = "default_route_cache_ttl_secs")]
    pub route_cache_ttl_secs: u64,
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,
    #[serde(default = "default_circuit_breaker_cooldown_secs")]
    pub circuit_breaker_cooldown_secs: u64,
    #[serde(default = "default_dedup_cache_capacity")]
    pub dedup_cache_capacity: usize,
    #[serde(default = "default_gossip_ttl")]
    pub gossip_ttl: u8,
    #[serde(default)]
    pub allow_insecure_protected_store: bool,
    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,
    #[serde(default = "default_health_port")]
    pub health_port: u16,
    #[serde(default)]
    pub veilid: rekindle_types::config::VeilidNetworkConfig,
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
            metrics_port: default_metrics_port(),
            health_port: default_health_port(),
            veilid: rekindle_types::config::VeilidNetworkConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyUserConfig {
    #[serde(default = "SafetyProfileUser::default_text")]
    pub text: SafetyProfileUser,
    #[serde(default = "SafetyProfileUser::default_voice")]
    pub voice: SafetyProfileUser,
    #[serde(default = "SafetyProfileUser::default_dht")]
    pub dht: SafetyProfileUser,
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
    #[serde(default = "default_hop_count")]
    pub hop_count: u8,
    #[serde(default = "default_stability")]
    pub stability: String,
    #[serde(default = "default_sequencing")]
    pub sequencing: String,
}

impl SafetyProfileUser {
    pub fn default_text() -> Self { Self { hop_count: 1, stability: "reliable".into(), sequencing: "prefer_ordered".into() } }
    pub fn default_voice() -> Self { Self { hop_count: 1, stability: "low_latency".into(), sequencing: "no_preference".into() } }
    pub fn default_dht() -> Self { Self { hop_count: 1, stability: "reliable".into(), sequencing: "prefer_ordered".into() } }
    pub fn default_rpc() -> Self { Self { hop_count: 1, stability: "reliable".into(), sequencing: "ensure_ordered".into() } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub mouse: bool,
    #[serde(default = "default_true")]
    pub animations: bool,
    #[serde(default)]
    pub keybindings: HashMap<String, String>,
    #[serde(default = "default_tick_rate")]
    pub tick_rate: f64,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    pub min_hop_count: Option<u8>,
    #[serde(default)]
    pub require_signature_verification: bool,
    pub max_gossip_ttl: Option<u8>,
}

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
fn default_metrics_port() -> u16 { 9191 }
fn default_health_port() -> u16 { 9192 }
fn default_hop_count() -> u8 { 1 }
fn default_stability() -> String { "reliable".into() }
fn default_sequencing() -> String { "prefer_ordered".into() }
