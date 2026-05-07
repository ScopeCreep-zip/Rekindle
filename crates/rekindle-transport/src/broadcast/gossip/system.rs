//! System gossip: announcements, raid alerts, lockdowns, kicked notifications.

use parking_lot::RwLock;

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::payload::gossip::ControlPayload;
use super::helpers::{control, MeshMap};

pub async fn system_message(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, body: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::SystemMessage {
            body: body.into(), timestamp: rekindle_utils::timestamp_ms(),
        }).await
}

pub async fn raid_alert(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, active: bool, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::RaidAlert { active }).await
}

pub async fn channel_lockdown(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, locked: bool, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::ChannelLockdown { locked }).await
}

pub async fn kicked_notification(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::KickedNotification).await
}
