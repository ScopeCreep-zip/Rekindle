//! Bootstrap and channel-sync request/response broadcasts.

use parking_lot::RwLock;

use super::super::node::TransportNode;
use super::super::send::BroadcastReport;
use super::{control, MeshMap};
use crate::payload::gossip::ControlPayload;

pub async fn bootstrap_request(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    joiner_pseudonym: &str,
    governance_key: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::BootstrapRequest {
            joiner_pseudonym: joiner_pseudonym.into(),
            governance_key: governance_key.into(),
        },
    )
    .await
}

pub async fn bootstrap_response(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    governance_entries: Vec<Vec<u8>>,
    member_list: Vec<Vec<u8>>,
    channel_meks: Vec<Vec<u8>>,
    recent_messages: Vec<Vec<u8>>,
    wrapped_owner_keypair: Vec<u8>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::BootstrapResponse {
            governance_entries,
            member_list,
            channel_meks,
            recent_messages,
            wrapped_owner_keypair,
        },
    )
    .await
}

pub async fn sync_request(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    since_timestamp: u64,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SyncRequest {
            channel_id: channel_id.into(),
            since_timestamp,
        },
    )
    .await
}

pub async fn sync_response(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    messages: Vec<Vec<u8>>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SyncResponse {
            channel_id: channel_id.into(),
            messages,
        },
    )
    .await
}
