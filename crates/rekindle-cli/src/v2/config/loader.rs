//! 8-layer config loading with deep merge and policy enforcement.

use std::path::{Path, PathBuf};
use anyhow::Context;
use super::schema::{Config, PolicyConfig};

/// Load config from all layers, merge, and enforce policy.
pub fn load(override_path: Option<&Path>) -> anyhow::Result<Config> {
    let mut config = Config::default();

    if let Some(layer) = load_layer(&PathBuf::from("/etc/rekindle/config.toml"))? {
        merge(&mut config, &layer);
    }
    for dropin in collect_dropins(&PathBuf::from("/etc/rekindle/config.d"))? {
        merge(&mut config, &dropin);
    }

    let policy = load_policy(&PathBuf::from("/etc/rekindle/policy.toml"))?;

    let user_dir = crate::v2::helpers::config_dir()?;
    if let Some(layer) = load_layer(&user_dir.join("config.toml"))? {
        merge(&mut config, &layer);
    }
    for dropin in collect_dropins(&user_dir.join("config.d"))? {
        merge(&mut config, &dropin);
    }

    if let Ok(env_path) = std::env::var("REKINDLE_CONFIG") {
        if let Some(layer) = load_layer(&PathBuf::from(&env_path))? {
            merge(&mut config, &layer);
        }
    }

    if let Some(path) = override_path {
        let layer = load_layer(path)?
            .ok_or_else(|| anyhow::anyhow!("config file not found: {}", path.display()))?;
        merge(&mut config, &layer);
    }

    if let Some(ref policy) = policy {
        enforce_policy(&config, policy)?;
    }

    Ok(config)
}

pub fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/etc/rekindle/config.toml"),
        PathBuf::from("/etc/rekindle/config.d/"),
        PathBuf::from("/etc/rekindle/policy.toml"),
    ];
    if let Ok(user_dir) = crate::v2::helpers::config_dir() {
        paths.push(user_dir.join("config.toml"));
        paths.push(user_dir.join("config.d/"));
    }
    if let Ok(env_path) = std::env::var("REKINDLE_CONFIG") {
        paths.push(PathBuf::from(env_path));
    }
    paths
}

fn load_layer(path: &Path) -> anyhow::Result<Option<Config>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let config: Config = toml::from_str(&contents)
                .with_context(|| format!("failed to parse config: {}", path.display()))?;
            Ok(Some(config))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to read config {}: {e}", path.display())),
    }
}

fn load_policy(path: &Path) -> anyhow::Result<Option<PolicyConfig>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let policy: PolicyConfig = toml::from_str(&contents)
                .with_context(|| format!("failed to parse policy: {}", path.display()))?;
            Ok(Some(policy))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to read policy {}: {e}", path.display())),
    }
}

fn collect_dropins(dir: &Path) -> anyhow::Result<Vec<Config>> {
    if !dir.is_dir() { return Ok(Vec::new()); }

    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read drop-in directory: {}", dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();

    let mut configs = Vec::new();
    for path in &paths {
        if let Some(config) = load_layer(path)? {
            configs.push(config);
        }
    }
    Ok(configs)
}

fn merge(base: &mut Config, overlay: &Config) {
    if overlay.config_version > base.config_version {
        base.config_version = overlay.config_version;
    }
    if !overlay.global.namespace.is_empty() && overlay.global.namespace != "rekindle" {
        base.global.namespace.clone_from(&overlay.global.namespace);
    }
    if overlay.global.default_community.is_some() {
        base.global.default_community.clone_from(&overlay.global.default_community);
    }
    merge_network(&mut base.network, &overlay.network);
    merge_tui(&mut base.tui, &overlay.tui);
}

fn merge_network(base: &mut super::schema::NetworkConfig, overlay: &super::schema::NetworkConfig) {
    if overlay.rpc_timeout_ms != 8_000 { base.rpc_timeout_ms = overlay.rpc_timeout_ms; }
    if overlay.dht_write_retries != 3 { base.dht_write_retries = overlay.dht_write_retries; }
    if overlay.route_refresh_secs != 60 { base.route_refresh_secs = overlay.route_refresh_secs; }
    if overlay.route_cache_ttl_secs != 90 { base.route_cache_ttl_secs = overlay.route_cache_ttl_secs; }
    if overlay.circuit_breaker_threshold != 3 { base.circuit_breaker_threshold = overlay.circuit_breaker_threshold; }
    if overlay.circuit_breaker_cooldown_secs != 45 { base.circuit_breaker_cooldown_secs = overlay.circuit_breaker_cooldown_secs; }
    if overlay.gossip_ttl != 5 { base.gossip_ttl = overlay.gossip_ttl; }
    if overlay.allow_insecure_protected_store { base.allow_insecure_protected_store = true; }
}

fn merge_tui(base: &mut super::schema::TuiConfig, overlay: &super::schema::TuiConfig) {
    if overlay.theme != "catppuccin-latte" { base.theme.clone_from(&overlay.theme); }
    if !overlay.mouse { base.mouse = false; }
    if !overlay.animations { base.animations = false; }
    #[allow(clippy::float_cmp)]
    if overlay.tick_rate != 4.0 { base.tick_rate = overlay.tick_rate; }
    #[allow(clippy::float_cmp)]
    if overlay.frame_rate != 30.0 { base.frame_rate = overlay.frame_rate; }
    for (key, value) in &overlay.keybindings {
        base.keybindings.insert(key.clone(), value.clone());
    }
}

fn enforce_policy(config: &Config, policy: &PolicyConfig) -> anyhow::Result<()> {
    if let Some(min_hops) = policy.min_hop_count {
        let profiles = [
            ("text", config.network.safety.text.hop_count),
            ("voice", config.network.safety.voice.hop_count),
            ("dht", config.network.safety.dht.hop_count),
            ("rpc", config.network.safety.rpc.hop_count),
        ];
        for (name, hops) in profiles {
            if hops < min_hops {
                anyhow::bail!(
                    "policy violation: {name} hop_count={hops} is below minimum {min_hops}\n\
                     set by admin in /etc/rekindle/policy.toml"
                );
            }
        }
    }
    if let Some(max_ttl) = policy.max_gossip_ttl {
        if config.network.gossip_ttl > max_ttl {
            anyhow::bail!(
                "policy violation: gossip_ttl={} exceeds maximum {max_ttl}\n\
                 set by admin in /etc/rekindle/policy.toml",
                config.network.gossip_ttl
            );
        }
    }
    Ok(())
}
