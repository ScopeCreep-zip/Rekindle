//! Message edit/delete, reaction, and pin broadcasts.

use parking_lot::RwLock;

use super::super::node::TransportNode;
use super::super::send::BroadcastReport;
use super::{control, MeshMap};
use crate::payload::gossip::ControlPayload;

pub async fn message_edited(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    new_ciphertext: Vec<u8>,
    mek_generation: u64,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessageEdited {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            new_ciphertext,
            mek_generation,
            edited_at: rekindle_utils::timestamp_ms(),
        },
    )
    .await
}

pub async fn message_deleted(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessageDeleted {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
        },
    )
    .await
}

pub async fn reaction_added(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    emoji: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ReactionAdded {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            emoji: emoji.into(),
            reactor_pseudonym: sender.into(),
        },
    )
    .await
}

pub async fn reaction_removed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    emoji: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ReactionRemoved {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            emoji: emoji.into(),
            reactor_pseudonym: sender.into(),
        },
    )
    .await
}

pub async fn message_pinned(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessagePinned {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            pinned_by: sender.into(),
        },
    )
    .await
}

pub async fn message_unpinned(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessageUnpinned {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
        },
    )
    .await
}
