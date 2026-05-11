//! Semantic config validation — value ranges, cross-field dependencies.

use super::schema::Config;

pub fn validate(config: &Config) -> anyhow::Result<()> {
    validate_namespace(&config.global.namespace)?;
    validate_network(&config.network)?;
    validate_tui(&config.tui)?;
    Ok(())
}

fn validate_namespace(namespace: &str) -> anyhow::Result<()> {
    if namespace.is_empty() { anyhow::bail!("global.namespace cannot be empty"); }
    if namespace.len() > 64 { anyhow::bail!("global.namespace too long ({} chars, max 64)", namespace.len()); }
    if !namespace.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("global.namespace may only contain alphanumeric characters, hyphens, and underscores");
    }
    Ok(())
}

fn validate_network(network: &super::schema::NetworkConfig) -> anyhow::Result<()> {
    if network.rpc_timeout_ms == 0 { anyhow::bail!("network.rpc_timeout_ms must be > 0"); }
    if network.rpc_timeout_ms > 300_000 { anyhow::bail!("network.rpc_timeout_ms too large ({}, max 300000)", network.rpc_timeout_ms); }
    if network.gossip_ttl == 0 { anyhow::bail!("network.gossip_ttl must be > 0"); }
    if network.gossip_ttl > 10 { anyhow::bail!("network.gossip_ttl too large ({}, max 10)", network.gossip_ttl); }
    if network.circuit_breaker_threshold == 0 { anyhow::bail!("network.circuit_breaker_threshold must be > 0"); }
    if network.route_refresh_secs == 0 { anyhow::bail!("network.route_refresh_secs must be > 0"); }
    if network.route_cache_ttl_secs == 0 { anyhow::bail!("network.route_cache_ttl_secs must be > 0"); }

    for (name, profile) in [
        ("text", &network.safety.text),
        ("voice", &network.safety.voice),
        ("dht", &network.safety.dht),
        ("rpc", &network.safety.rpc),
    ] {
        validate_safety_profile(name, profile)?;
    }
    Ok(())
}

fn validate_safety_profile(name: &str, profile: &super::schema::SafetyProfileUser) -> anyhow::Result<()> {
    if profile.hop_count == 0 {
        anyhow::bail!("network.safety.{name}.hop_count cannot be 0 (disables anonymity routing, exposes real IP)");
    }
    if profile.hop_count > 4 { anyhow::bail!("network.safety.{name}.hop_count too large ({}, max 4)", profile.hop_count); }
    let valid_stability = ["low_latency", "reliable"];
    if !valid_stability.contains(&profile.stability.as_str()) {
        anyhow::bail!("network.safety.{name}.stability must be one of: {}", valid_stability.join(", "));
    }
    let valid_sequencing = ["no_preference", "prefer_ordered", "ensure_ordered"];
    if !valid_sequencing.contains(&profile.sequencing.as_str()) {
        anyhow::bail!("network.safety.{name}.sequencing must be one of: {}", valid_sequencing.join(", "));
    }
    Ok(())
}

fn validate_tui(tui: &super::schema::TuiConfig) -> anyhow::Result<()> {
    if tui.theme.is_empty() { anyhow::bail!("tui.theme cannot be empty"); }
    if tui.tick_rate <= 0.0 { anyhow::bail!("tui.tick_rate must be positive"); }
    if tui.tick_rate > 60.0 { anyhow::bail!("tui.tick_rate too high ({}, max 60)", tui.tick_rate); }
    if tui.frame_rate <= 0.0 { anyhow::bail!("tui.frame_rate must be positive"); }
    if tui.frame_rate > 120.0 { anyhow::bail!("tui.frame_rate too high ({}, max 120)", tui.frame_rate); }
    Ok(())
}
