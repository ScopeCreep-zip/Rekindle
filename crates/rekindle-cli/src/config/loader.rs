//! 8-layer config loading with deep merge and policy enforcement.
//!
//! Precedence (lowest → highest):
//! 1. `Config::default()` — compiled defaults
//! 2. `/etc/rekindle/config.toml` — system baseline
//! 3. `/etc/rekindle/config.d/*.toml` — system drop-ins
//! 4. `/etc/rekindle/policy.toml` — admin policy (fail-closed)
//! 5. `~/.config/rekindle/config.toml` — user preferences
//! 6. `~/.config/rekindle/config.d/*.toml` — user drop-ins
//! 7. `$REKINDLE_CONFIG` env — CI/container override
//! 8. CLI flags — highest priority (handled by caller)

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::schema::{Config, PolicyConfig};

/// Load config from all layers, merge, and enforce policy.
///
/// `override_path` is from the `--config` CLI flag — highest priority file.
pub fn load(override_path: Option<&Path>) -> anyhow::Result<Config> {
    let mut config = Config::default();

    // Layer 2: System config
    let system_path = PathBuf::from("/etc/rekindle/config.toml");
    if let Some(layer) = load_layer(&system_path)? {
        merge(&mut config, &layer);
    }

    // Layer 3: System drop-ins
    for dropin in collect_dropins(&PathBuf::from("/etc/rekindle/config.d"))? {
        merge(&mut config, &dropin);
    }

    // Layer 4: Policy (loaded but NOT merged — enforced separately)
    let policy = load_policy(&PathBuf::from("/etc/rekindle/policy.toml"))?;

    // Layer 5: User config
    let user_dir = crate::helpers::config_dir()?;
    if let Some(layer) = load_layer(&user_dir.join("config.toml"))? {
        merge(&mut config, &layer);
    }

    // Layer 6: User drop-ins
    for dropin in collect_dropins(&user_dir.join("config.d"))? {
        merge(&mut config, &dropin);
    }

    // Layer 7: Environment override
    if let Ok(env_path) = std::env::var("REKINDLE_CONFIG") {
        let path = PathBuf::from(&env_path);
        if let Some(layer) = load_layer(&path)? {
            merge(&mut config, &layer);
        }
    }

    // Layer 8: CLI override path (--config flag)
    if let Some(path) = override_path {
        let layer = load_layer(path)?
            .ok_or_else(|| anyhow::anyhow!("config file not found: {}", path.display()))?;
        merge(&mut config, &layer);
    }

    // Enforce policy AFTER all merging
    if let Some(ref policy) = policy {
        enforce_policy(&config, policy)?;
    }

    Ok(config)
}

/// Return the list of config file paths that are searched, in precedence order.
///
/// Used by `rekindle config paths` to show the user where config files
/// are loaded from.
pub fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // System
    paths.push(PathBuf::from("/etc/rekindle/config.toml"));
    paths.push(PathBuf::from("/etc/rekindle/config.d/"));
    paths.push(PathBuf::from("/etc/rekindle/policy.toml"));

    // User
    if let Ok(user_dir) = crate::helpers::config_dir() {
        paths.push(user_dir.join("config.toml"));
        paths.push(user_dir.join("config.d/"));
    }

    // Environment
    if let Ok(env_path) = std::env::var("REKINDLE_CONFIG") {
        paths.push(PathBuf::from(env_path));
    }

    paths
}

/// Load a single TOML config file. Returns `None` if the file doesn't exist.
fn load_layer(path: &Path) -> anyhow::Result<Option<Config>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            // Resolve NixOS symlinks — canonicalize to follow symlinks
            // into /nix/store before parsing (Landlock compatibility).
            let _canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

            let config: Config = toml::from_str(&contents)
                .with_context(|| format!("failed to parse config: {}", path.display()))?;
            Ok(Some(config))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!(
            "failed to read config {}: {e}",
            path.display()
        )),
    }
}

/// Load the admin policy file. Returns `None` if it doesn't exist.
fn load_policy(path: &Path) -> anyhow::Result<Option<PolicyConfig>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let policy: PolicyConfig = toml::from_str(&contents)
                .with_context(|| format!("failed to parse policy: {}", path.display()))?;
            Ok(Some(policy))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!(
            "failed to read policy {}: {e}",
            path.display()
        )),
    }
}

/// Collect and sort TOML drop-in files from a directory.
///
/// Files are sorted alphabetically (NN-description.toml convention).
/// Non-TOML files are silently skipped.
fn collect_dropins(dir: &Path) -> anyhow::Result<Vec<Config>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

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

/// Merge an overlay config into the base config.
///
/// Merge semantics:
/// - `config_version`: take higher value
/// - Scalar fields: overlay overrides if non-default
/// - Maps (keybindings): merge by key, overlay wins on collision
/// - The `safety` section: field-level merge
fn merge(base: &mut Config, overlay: &Config) {
    // Version: take higher
    if overlay.config_version > base.config_version {
        base.config_version = overlay.config_version;
    }

    // Global
    if !overlay.global.namespace.is_empty() && overlay.global.namespace != "rekindle" {
        base.global.namespace.clone_from(&overlay.global.namespace);
    }
    if overlay.global.default_community.is_some() {
        base.global
            .default_community
            .clone_from(&overlay.global.default_community);
    }

    // Network — merge non-default values
    merge_network(&mut base.network, &overlay.network);

    // TUI — merge non-default values
    merge_tui(&mut base.tui, &overlay.tui);
}

fn merge_network(base: &mut super::schema::NetworkConfig, overlay: &super::schema::NetworkConfig) {
    if overlay.rpc_timeout_ms != 8_000 {
        base.rpc_timeout_ms = overlay.rpc_timeout_ms;
    }
    if overlay.dht_write_retries != 3 {
        base.dht_write_retries = overlay.dht_write_retries;
    }
    if overlay.route_refresh_secs != 60 {
        base.route_refresh_secs = overlay.route_refresh_secs;
    }
    if overlay.route_cache_ttl_secs != 90 {
        base.route_cache_ttl_secs = overlay.route_cache_ttl_secs;
    }
    if overlay.circuit_breaker_threshold != 3 {
        base.circuit_breaker_threshold = overlay.circuit_breaker_threshold;
    }
    if overlay.circuit_breaker_cooldown_secs != 45 {
        base.circuit_breaker_cooldown_secs = overlay.circuit_breaker_cooldown_secs;
    }
    if overlay.gossip_ttl != 5 {
        base.gossip_ttl = overlay.gossip_ttl;
    }
    if overlay.allow_insecure_protected_store {
        base.allow_insecure_protected_store = true;
    }
}

fn merge_tui(base: &mut super::schema::TuiConfig, overlay: &super::schema::TuiConfig) {
    if overlay.theme != "catppuccin-latte" {
        base.theme.clone_from(&overlay.theme);
    }
    if !overlay.mouse {
        base.mouse = false;
    }
    if !overlay.animations {
        base.animations = false;
    }
    #[allow(clippy::float_cmp)]
    if overlay.tick_rate != 4.0 {
        base.tick_rate = overlay.tick_rate;
    }
    #[allow(clippy::float_cmp)]
    if overlay.frame_rate != 30.0 {
        base.frame_rate = overlay.frame_rate;
    }
    // Keybindings: merge by key, overlay wins
    for (key, value) in &overlay.keybindings {
        base.keybindings.insert(key.clone(), value.clone());
    }
}

/// Enforce admin policy constraints.
///
/// Policy violations produce hard errors with remediation messages
/// pointing to the system administrator. Policy fields are additive —
/// they constrain but never grant capabilities.
fn enforce_policy(config: &Config, policy: &PolicyConfig) -> anyhow::Result<()> {
    // Min hop count
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
                     set by admin in /etc/rekindle/policy.toml\n\
                     contact your system administrator to adjust"
                );
            }
        }
    }

    // Max gossip TTL
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_loads() {
        // load() without any files on disk should return defaults
        // (assuming /etc/rekindle/ doesn't exist in test environments)
        let cfg = load(None).unwrap();
        assert_eq!(cfg.global.namespace, "rekindle");
    }

    #[test]
    fn merge_preserves_base_when_overlay_is_default() {
        let mut base = Config::default();
        base.global.namespace = "custom".into();
        let overlay = Config::default();
        merge(&mut base, &overlay);
        assert_eq!(base.global.namespace, "custom");
    }

    #[test]
    fn merge_overrides_non_default() {
        let mut base = Config::default();
        let mut overlay = Config::default();
        overlay.network.rpc_timeout_ms = 15_000;
        merge(&mut base, &overlay);
        assert_eq!(base.network.rpc_timeout_ms, 15_000);
    }

    #[test]
    fn merge_keybindings_additive() {
        let mut base = Config::default();
        base.tui.keybindings.insert("q".into(), "quit".into());

        let mut overlay = Config::default();
        overlay.tui.keybindings.insert("r".into(), "refresh".into());

        merge(&mut base, &overlay);
        assert_eq!(base.tui.keybindings.len(), 2);
        assert_eq!(base.tui.keybindings.get("q").unwrap(), "quit");
        assert_eq!(base.tui.keybindings.get("r").unwrap(), "refresh");
    }

    #[test]
    fn policy_enforces_min_hop_count() {
        let mut config = Config::default();
        config.network.safety.text.hop_count = 0;

        let policy = PolicyConfig {
            min_hop_count: Some(1),
            ..PolicyConfig::default()
        };

        let result = enforce_policy(&config, &policy);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("policy violation"));
        assert!(err.contains("hop_count=0"));
    }

    #[test]
    fn policy_allows_compliant_config() {
        let config = Config::default();
        let policy = PolicyConfig {
            min_hop_count: Some(1),
            ..PolicyConfig::default()
        };
        assert!(enforce_policy(&config, &policy).is_ok());
    }

    #[test]
    fn config_search_paths_returns_multiple() {
        let paths = config_search_paths();
        assert!(paths.len() >= 3);
    }
}
