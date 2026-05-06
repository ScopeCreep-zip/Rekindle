//! Moderation operations — kick, ban, unban, timeout.
//!
//! Typed reads/writes via `dht/governance.rs`.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::BanEntry;
use crate::payload::gossip::{ControlPayload, GossipPayload};

pub fn build_kick_payload(target_pseudonym: &str) -> Result<Vec<u8>> {
    serialize_control(ControlPayload::Kick { target_pseudonym: target_pseudonym.to_string() })
}

pub async fn ban_member(
    node: &TransportNode, governance_key: &str,
    target_pseudonym: &str, reason: Option<&str>, banned_by: &str,
) -> Result<Vec<u8>> {
    let dht = node.dht()?;
    let mut bans = dht.governance().read_bans(governance_key).await?;
    if bans.iter().any(|b| b.pseudonym_key == target_pseudonym) {
        return Err(TransportError::DhtError { reason: format!("{target_pseudonym} already banned") });
    }
    bans.push(BanEntry {
        pseudonym_key: target_pseudonym.to_string(), reason: reason.map(String::from),
        banned_by: banned_by.to_string(), banned_at: rekindle_utils::timestamp_ms(),
    });
    dht.governance().write_bans(governance_key, &bans).await?;
    info!(target = target_pseudonym, "member banned");
    serialize_control(ControlPayload::Ban { target_pseudonym: target_pseudonym.to_string() })
}

pub async fn unban_member(node: &TransportNode, governance_key: &str, target_pseudonym: &str) -> Result<Vec<u8>> {
    let dht = node.dht()?;
    let mut bans = dht.governance().read_bans(governance_key).await?;
    let before = bans.len();
    bans.retain(|b| b.pseudonym_key != target_pseudonym);
    if bans.len() == before {
        return Err(TransportError::DhtError { reason: format!("{target_pseudonym} not banned") });
    }
    dht.governance().write_bans(governance_key, &bans).await?;
    info!(target = target_pseudonym, "member unbanned");
    serialize_control(ControlPayload::Unban { target_pseudonym: target_pseudonym.to_string() })
}

pub fn build_timeout_payload(target_pseudonym: &str, duration_seconds: u64, reason: Option<&str>) -> Result<Vec<u8>> {
    serialize_control(ControlPayload::TimeoutMember {
        target_pseudonym: target_pseudonym.to_string(), duration_seconds, reason: reason.map(String::from),
    })
}

pub async fn list_bans(node: &TransportNode, governance_key: &str) -> Result<Vec<BanEntry>> {
    node.dht()?.governance().read_bans(governance_key).await
}

fn serialize_control(ctrl: ControlPayload) -> Result<Vec<u8>> {
    postcard::to_stdvec(&GossipPayload::Control(ctrl))
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}
