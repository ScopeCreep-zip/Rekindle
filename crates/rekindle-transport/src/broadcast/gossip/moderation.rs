//! Moderation gossip: kick, ban, unban, timeout, remove timeout.

use parking_lot::RwLock;

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::payload::gossip::ControlPayload;
use super::helpers::{control, MeshMap};

pub async fn kick(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, target: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::Kick { target_pseudonym: target.into() }).await
}

pub async fn ban(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, target: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::Ban { target_pseudonym: target.into() }).await
}

pub async fn unban(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, target: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::Unban { target_pseudonym: target.into() }).await
}

pub async fn timeout_member(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, target: &str,
    duration_seconds: u64, reason: Option<&str>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::TimeoutMember {
            target_pseudonym: target.into(), duration_seconds,
            reason: reason.map(String::from),
        }).await
}

pub async fn remove_timeout(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, target: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::RemoveTimeout { target_pseudonym: target.into() }).await
}

pub async fn member_timed_out(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, pseudonym: &str,
    timeout_until: Option<u64>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::MemberTimedOut { pseudonym_key: pseudonym.into(), timeout_until }).await
}
