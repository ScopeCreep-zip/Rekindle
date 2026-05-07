//! Message gossip: notification, edited, deleted.

use parking_lot::RwLock;

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::payload::gossip::{ControlPayload, GossipPayload};
use super::helpers::{build_sign_send, control, MeshMap};

/// Broadcast a `MessageNotification` after appending to a channel DhtLog.
pub async fn message_notification(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, channel_id: &str, message_id: &str,
    author_pseudonym: &str, sequence: u64, content_hash: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::MessageNotification {
        channel_id: channel_id.into(), message_id: message_id.into(),
        author_pseudonym: author_pseudonym.into(), subkey_index: 0,
        lamport_ts: 0, sequence, content_hash: content_hash.into(),
        timestamp: rekindle_utils::timestamp_ms(),
    };
    build_sign_send(node, meshes, community_id, author_pseudonym, signing_key, payload).await
}

pub async fn message_edited(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str, message_id: &str,
    new_ciphertext: Vec<u8>, mek_generation: u64, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::MessageEdited {
            channel_id: channel_id.into(), message_id: message_id.into(),
            new_ciphertext, mek_generation,
            edited_at: rekindle_utils::timestamp_ms(),
        }).await
}

pub async fn message_deleted(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    message_id: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::MessageDeleted {
            channel_id: channel_id.into(), message_id: message_id.into(),
        }).await
}
