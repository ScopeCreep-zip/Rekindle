//! The host's transport config (plan D3.1): the `[network]` section of the
//! layered `config.toml` `rekindled` starts its node with, and the admin
//! policy's ceilings and floors applied to whichever config a host starts.

use rekindle_transport::TransportConfig;

use super::{policy, ConfigFault};

/// Apply the admin policy's ceilings and floors to the transport config.
/// `min_hop_count` is a floor on the safety-route length, beside the
/// transport's own `ANONYMITY_HOP_FLOOR`: an administrator can raise it,
/// never lower it.
pub(super) fn apply_policy(mut config: TransportConfig, policy: &policy::PolicyConfig) -> TransportConfig {
    if let Some(ceiling) = policy.max_gossip_ttl {
        if config.gossip_ttl > ceiling {
            tracing::info!(gossip_ttl = ceiling, "admin policy capped the gossip TTL");
            config.gossip_ttl = ceiling;
        }
    }
    if let Some(floor) = policy.min_hop_count {
        for profile in [
            &mut config.safety.text,
            &mut config.safety.voice,
            &mut config.safety.dht,
            &mut config.safety.rpc,
        ] {
            profile.hop_count = profile.hop_count.max(floor);
        }
        tracing::info!(
            min_hop_count = floor,
            "admin policy raised the safety-route floor"
        );
    }
    config
}

/// The `[network]` section of the layered `config.toml`, the file and schema
/// `rekindle config validate` checks: `rekindled`'s transport config.
///
/// # Errors
/// The file does not parse or validate (`ConfigFault`, `EX_CONFIG`).
pub fn user_transport_config() -> anyhow::Result<TransportConfig> {
    let user_config: rekindle_types::config::user::UserConfig =
        rekindle_utils::config_layers::load(None).map_err(|e| ConfigFault(e.into()))?;
    user_config.validate().map_err(|e| ConfigFault(e.into()))?;
    Ok(user_config.network)
}
