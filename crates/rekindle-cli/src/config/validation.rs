//! Semantic config validation.
//!
//! Checks constraints that serde can't express: value ranges, path
//! accessibility, cross-field dependencies. Called after loading and
//! merging all config layers.

use super::schema::Config;

/// Validate the fully-merged config.
///
/// Returns `Ok(())` if all semantic constraints are satisfied.
/// Returns `Err` with a descriptive message for the first violation found.
pub fn validate(config: &Config) -> anyhow::Result<()> {
    validate_namespace(&config.global.namespace)?;
    validate_network(&config.network)?;
    validate_tui(&config.tui)?;
    Ok(())
}

fn validate_namespace(namespace: &str) -> anyhow::Result<()> {
    if namespace.is_empty() {
        anyhow::bail!("global.namespace cannot be empty");
    }
    if namespace.len() > 64 {
        anyhow::bail!(
            "global.namespace too long ({} chars, max 64)",
            namespace.len()
        );
    }
    if !namespace
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "global.namespace may only contain alphanumeric characters, hyphens, and underscores"
        );
    }
    Ok(())
}

fn validate_network(network: &super::schema::NetworkConfig) -> anyhow::Result<()> {
    if network.rpc_timeout_ms == 0 {
        anyhow::bail!("network.rpc_timeout_ms must be > 0");
    }
    if network.rpc_timeout_ms > 300_000 {
        anyhow::bail!(
            "network.rpc_timeout_ms too large ({}, max 300000)",
            network.rpc_timeout_ms
        );
    }
    if network.gossip_ttl == 0 {
        anyhow::bail!("network.gossip_ttl must be > 0");
    }
    if network.gossip_ttl > 10 {
        anyhow::bail!(
            "network.gossip_ttl too large ({}, max 10)",
            network.gossip_ttl
        );
    }
    if network.circuit_breaker_threshold == 0 {
        anyhow::bail!("network.circuit_breaker_threshold must be > 0");
    }
    if network.route_refresh_secs == 0 {
        anyhow::bail!("network.route_refresh_secs must be > 0");
    }
    if network.route_cache_ttl_secs == 0 {
        anyhow::bail!("network.route_cache_ttl_secs must be > 0");
    }

    // Validate safety profiles
    validate_safety_profile("text", &network.safety.text)?;
    validate_safety_profile("voice", &network.safety.voice)?;
    validate_safety_profile("dht", &network.safety.dht)?;
    validate_safety_profile("rpc", &network.safety.rpc)?;

    Ok(())
}

fn validate_safety_profile(
    name: &str,
    profile: &super::schema::SafetyProfileUser,
) -> anyhow::Result<()> {
    if profile.hop_count > 4 {
        anyhow::bail!(
            "network.safety.{name}.hop_count too large ({}, max 4)",
            profile.hop_count
        );
    }
    let valid_stability = ["low_latency", "reliable"];
    if !valid_stability.contains(&profile.stability.as_str()) {
        anyhow::bail!(
            "network.safety.{name}.stability must be one of: {}",
            valid_stability.join(", ")
        );
    }
    let valid_sequencing = ["no_preference", "prefer_ordered", "ensure_ordered"];
    if !valid_sequencing.contains(&profile.sequencing.as_str()) {
        anyhow::bail!(
            "network.safety.{name}.sequencing must be one of: {}",
            valid_sequencing.join(", ")
        );
    }
    Ok(())
}

fn validate_tui(tui: &super::schema::TuiConfig) -> anyhow::Result<()> {
    if tui.theme.is_empty() {
        anyhow::bail!("tui.theme cannot be empty");
    }
    if tui.tick_rate <= 0.0 {
        anyhow::bail!("tui.tick_rate must be positive");
    }
    if tui.tick_rate > 60.0 {
        anyhow::bail!("tui.tick_rate too high ({}, max 60)", tui.tick_rate);
    }
    if tui.frame_rate <= 0.0 {
        anyhow::bail!("tui.frame_rate must be positive");
    }
    if tui.frame_rate > 120.0 {
        anyhow::bail!("tui.frame_rate too high ({}, max 120)", tui.frame_rate);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        let cfg = Config::default();
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn empty_namespace_rejected() {
        let mut cfg = Config::default();
        cfg.global.namespace = String::new();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn invalid_namespace_chars_rejected() {
        let mut cfg = Config::default();
        cfg.global.namespace = "hello world!".into();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn zero_rpc_timeout_rejected() {
        let mut cfg = Config::default();
        cfg.network.rpc_timeout_ms = 0;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn zero_gossip_ttl_rejected() {
        let mut cfg = Config::default();
        cfg.network.gossip_ttl = 0;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn excessive_gossip_ttl_rejected() {
        let mut cfg = Config::default();
        cfg.network.gossip_ttl = 11;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn empty_theme_rejected() {
        let mut cfg = Config::default();
        cfg.tui.theme = String::new();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn negative_tick_rate_rejected() {
        let mut cfg = Config::default();
        cfg.tui.tick_rate = -1.0;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn invalid_stability_rejected() {
        let mut cfg = Config::default();
        cfg.network.safety.text.stability = "garbage".into();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn invalid_sequencing_rejected() {
        let mut cfg = Config::default();
        cfg.network.safety.rpc.sequencing = "garbage".into();
        assert!(validate(&cfg).is_err());
    }

    // ── Boundary values ────────────────────────────────────────────

    #[test]
    fn rpc_timeout_max_boundary() {
        let mut cfg = Config::default();
        cfg.network.rpc_timeout_ms = 300_000;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn rpc_timeout_over_max_rejected() {
        let mut cfg = Config::default();
        cfg.network.rpc_timeout_ms = 300_001;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn gossip_ttl_max_boundary() {
        let mut cfg = Config::default();
        cfg.network.gossip_ttl = 10;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn hop_count_max_boundary() {
        let mut cfg = Config::default();
        cfg.network.safety.text.hop_count = 4;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn hop_count_over_max_rejected() {
        let mut cfg = Config::default();
        cfg.network.safety.text.hop_count = 5;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn tick_rate_max_boundary() {
        let mut cfg = Config::default();
        cfg.tui.tick_rate = 60.0;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn tick_rate_over_max_rejected() {
        let mut cfg = Config::default();
        cfg.tui.tick_rate = 60.1;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn frame_rate_max_boundary() {
        let mut cfg = Config::default();
        cfg.tui.frame_rate = 120.0;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn frame_rate_over_max_rejected() {
        let mut cfg = Config::default();
        cfg.tui.frame_rate = 120.1;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn namespace_max_length() {
        let mut cfg = Config::default();
        cfg.global.namespace = "a".repeat(64);
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn namespace_over_max_rejected() {
        let mut cfg = Config::default();
        cfg.global.namespace = "a".repeat(65);
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn namespace_with_hyphens_and_underscores() {
        let mut cfg = Config::default();
        cfg.global.namespace = "my-cool_app".into();
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn namespace_with_special_chars_rejected() {
        let mut cfg = Config::default();
        cfg.global.namespace = "my.app".into();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn zero_circuit_breaker_threshold_rejected() {
        let mut cfg = Config::default();
        cfg.network.circuit_breaker_threshold = 0;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn zero_route_refresh_rejected() {
        let mut cfg = Config::default();
        cfg.network.route_refresh_secs = 0;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn zero_route_cache_ttl_rejected() {
        let mut cfg = Config::default();
        cfg.network.route_cache_ttl_secs = 0;
        assert!(validate(&cfg).is_err());
    }

    // ── All valid safety profiles accepted ──────────────────────────

    #[test]
    fn all_valid_stability_values() {
        for stability in ["low_latency", "reliable"] {
            let mut cfg = Config::default();
            cfg.network.safety.text.stability = stability.into();
            assert!(validate(&cfg).is_ok(), "stability '{stability}' should be valid");
        }
    }

    #[test]
    fn all_valid_sequencing_values() {
        for seq in ["no_preference", "prefer_ordered", "ensure_ordered"] {
            let mut cfg = Config::default();
            cfg.network.safety.text.sequencing = seq.into();
            assert!(validate(&cfg).is_ok(), "sequencing '{seq}' should be valid");
        }
    }

    // ── All four safety profiles validated independently ─────��──────

    #[test]
    fn all_four_safety_profiles_checked() {
        let profiles = ["text", "voice", "dht", "rpc"];
        for profile_name in profiles {
            let mut cfg = Config::default();
            let profile = match profile_name {
                "text" => &mut cfg.network.safety.text,
                "voice" => &mut cfg.network.safety.voice,
                "dht" => &mut cfg.network.safety.dht,
                "rpc" => &mut cfg.network.safety.rpc,
                _ => unreachable!(),
            };
            profile.hop_count = 5; // over max
            let result = validate(&cfg);
            assert!(
                result.is_err(),
                "hop_count=5 on {profile_name} should fail validation"
            );
        }
    }
}
