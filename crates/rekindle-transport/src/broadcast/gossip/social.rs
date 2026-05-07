//! Social gossip: reactions, pins, threads, events, game servers.

use parking_lot::RwLock;

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::payload::gossip::{ControlPayload, CommunityEvent, GameServerInfo, ThreadInfo};
use super::helpers::{control, MeshMap};

// ── Reactions ─────────────────────────────────────────────────────────

pub async fn reaction_added(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    message_id: &str, emoji: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::ReactionAdded {
            channel_id: channel_id.into(), message_id: message_id.into(),
            emoji: emoji.into(), reactor_pseudonym: sender.into(),
        }).await
}

pub async fn reaction_removed(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    message_id: &str, emoji: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::ReactionRemoved {
            channel_id: channel_id.into(), message_id: message_id.into(),
            emoji: emoji.into(), reactor_pseudonym: sender.into(),
        }).await
}

// ── Pins ──────────────────────────────────────────────────────────────

pub async fn message_pinned(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    message_id: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::MessagePinned {
            channel_id: channel_id.into(), message_id: message_id.into(),
            pinned_by: sender.into(),
        }).await
}

pub async fn message_unpinned(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    message_id: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::MessageUnpinned {
            channel_id: channel_id.into(), message_id: message_id.into(),
        }).await
}

// ── Events ────────────────────────────────────────────────────────────

pub async fn event_created(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, event: CommunityEvent, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::EventCreated { event }).await
}

pub async fn event_updated(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, event: CommunityEvent, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::EventUpdated { event }).await
}

pub async fn event_deleted(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, event_id: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::EventDeleted { event_id: event_id.into() }).await
}

pub async fn event_rsvp_changed(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, event_id: &str,
    rsvp_status: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::EventRsvpChanged {
            event_id: event_id.into(), pseudonym_key: sender.into(),
            status: rsvp_status.into(),
        }).await
}

pub async fn event_reminder(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, event_id: &str,
    title: &str, minutes_until_start: u32, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::EventReminder {
            event_id: event_id.into(), title: title.into(), minutes_until_start,
        }).await
}

// ── Threads ───────────────────────────────────────────────────────────

pub async fn thread_created(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, thread: ThreadInfo, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::ThreadCreated { thread }).await
}

pub async fn thread_message(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, thread_id: &str,
    message_id: &str, ciphertext: Vec<u8>, mek_generation: u64,
    reply_to_id: Option<&str>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::ThreadMessage {
            thread_id: thread_id.into(), message_id: message_id.into(),
            sender_pseudonym: sender.into(), ciphertext, mek_generation,
            timestamp: rekindle_utils::timestamp_ms(),
            reply_to_id: reply_to_id.map(String::from),
        }).await
}

pub async fn thread_archived(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, thread_id: &str,
    archived: bool, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::ThreadArchived { thread_id: thread_id.into(), archived }).await
}

// ── Game servers ──────────────────────────────────────────────────────

pub async fn game_server_added(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, server: GameServerInfo, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::GameServerAdded { server }).await
}

pub async fn game_server_removed(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, server_id: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::GameServerRemoved { server_id: server_id.into() }).await
}
