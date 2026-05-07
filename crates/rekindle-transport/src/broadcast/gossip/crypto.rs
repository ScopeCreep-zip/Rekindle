//! Crypto gossip: MEK rotation, request, transfer, admin/slot keypair grants.

use parking_lot::RwLock;

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::payload::gossip::ControlPayload;
use super::helpers::{control, MeshMap};

pub async fn mek_rotated(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: Option<&str>,
    new_generation: u64, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::MekRotated {
            channel_id: channel_id.map(String::from),
            new_generation, rotator_pseudonym: Some(sender.into()),
        }).await
}

pub async fn request_mek(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    needed_generation: u64, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::RequestMek {
            channel_id: channel_id.into(), needed_generation,
            requester_pseudonym: sender.into(),
        }).await
}

pub async fn mek_transfer(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: Option<&str>,
    generation: u64, wrapped_mek: Vec<u8>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::MekTransfer {
            community_id: community_id.into(),
            channel_id: channel_id.map(String::from),
            generation, sender_pseudonym: sender.into(), wrapped_mek,
        }).await
}

pub async fn admin_keypair_grant(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str,
    wrapped_owner_keypair: Vec<u8>, wrapped_slot_seed: Vec<u8>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::AdminKeypairGrant { wrapped_owner_keypair, wrapped_slot_seed }).await
}

pub async fn slot_keypair_grant(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, slot_index: u32,
    segment_index: u32, wrapped_slot_keypair: Vec<u8>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::SlotKeypairGrant {
            slot_index, segment_index, wrapped_slot_keypair,
        }).await
}
