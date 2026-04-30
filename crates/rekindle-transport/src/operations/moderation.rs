//! Moderation operations — kick, ban, unban, timeout.
//!
//! Each operation builds a serialized gossip control payload that the
//! CLI signs and broadcasts to the community mesh. Ban state is also
//! persisted to the governance bans subkey.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dht_types::BanEntry;
use crate::payload::gossip::{ControlPayload, GossipPayload};

/// Build a serialized `Kick` gossip payload. CLI signs and broadcasts.
pub fn build_kick_payload(target_pseudonym: &str) -> Result<Vec<u8>> {
    let payload = GossipPayload::Control(ControlPayload::Kick {
        target_pseudonym: target_pseudonym.to_string(),
    });
    postcard::to_stdvec(&payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

/// Ban a member: persist to governance bans subkey + build gossip payload.
///
/// Returns the serialized `Ban` gossip payload for the CLI to broadcast.
pub async fn ban_member(
    node: &TransportNode,
    governance_key: &str,
    target_pseudonym: &str,
    reason: Option<&str>,
    banned_by: &str,
) -> Result<Vec<u8>> {
    let dht = node.dht()?;

    // Add to bans list in governance
    let mut bans = dht.governance().read_bans(governance_key).await?;

    // Deduplicate
    if bans.iter().any(|b| b.pseudonym_key == target_pseudonym) {
        return Err(TransportError::DhtError {
            reason: format!("member {target_pseudonym} is already banned"),
        });
    }

    bans.push(BanEntry {
        pseudonym_key: target_pseudonym.to_string(),
        reason: reason.map(String::from),
        banned_by: banned_by.to_string(),
        banned_at: rekindle_utils::timestamp_ms(),
    });

    dht.governance().write_bans(governance_key, &bans).await?;

    info!(target = target_pseudonym, "member banned");

    // Build gossip payload
    let payload = GossipPayload::Control(ControlPayload::Ban {
        target_pseudonym: target_pseudonym.to_string(),
    });
    postcard::to_stdvec(&payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

/// Unban a member: remove from governance bans subkey + build gossip payload.
pub async fn unban_member(
    node: &TransportNode,
    governance_key: &str,
    target_pseudonym: &str,
) -> Result<Vec<u8>> {
    let dht = node.dht()?;

    let mut bans = dht.governance().read_bans(governance_key).await?;
    let before = bans.len();
    bans.retain(|b| b.pseudonym_key != target_pseudonym);

    if bans.len() == before {
        return Err(TransportError::DhtError {
            reason: format!("member {target_pseudonym} is not banned"),
        });
    }

    dht.governance().write_bans(governance_key, &bans).await?;

    info!(target = target_pseudonym, "member unbanned");

    let payload = GossipPayload::Control(ControlPayload::Unban {
        target_pseudonym: target_pseudonym.to_string(),
    });
    postcard::to_stdvec(&payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

/// Build a serialized `TimeoutMember` gossip payload.
pub fn build_timeout_payload(
    target_pseudonym: &str,
    duration_seconds: u64,
    reason: Option<&str>,
) -> Result<Vec<u8>> {
    let payload = GossipPayload::Control(ControlPayload::TimeoutMember {
        target_pseudonym: target_pseudonym.to_string(),
        duration_seconds,
        reason: reason.map(String::from),
    });
    postcard::to_stdvec(&payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

/// List all active bans for a community.
pub async fn list_bans(
    node: &TransportNode,
    governance_key: &str,
) -> Result<Vec<BanEntry>> {
    let dht = node.dht()?;
    dht.governance().read_bans(governance_key).await
}
