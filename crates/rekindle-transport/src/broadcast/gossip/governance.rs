//! Governance gossip: governance updated, channel overwrite changed.

use parking_lot::RwLock;

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::payload::gossip::ControlPayload;
use super::helpers::{control, MeshMap};

pub async fn governance_updated(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, governance_key: &str,
    subkey_index: u32, lamport_ts: u64, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::GovernanceUpdated {
            governance_key: governance_key.into(), subkey_index, lamport_ts,
        }).await
}

pub async fn channel_overwrite_changed(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::ChannelOverwriteChanged { channel_id: channel_id.into() }).await
}
