//! Voice signaling gossip: join, leave, mode switch, mute, deafen, roster.

use parking_lot::RwLock;

use crate::broadcast::node::TransportNode;
use crate::broadcast::send::BroadcastReport;
use crate::payload::gossip::{ControlPayload, VoiceParticipant};
use super::helpers::{control, MeshMap};

pub async fn voice_join(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    route_blob: Vec<u8>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::VoiceJoin { channel_id: channel_id.into(), route_blob }).await
}

pub async fn voice_leave(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::VoiceLeave { channel_id: channel_id.into() }).await
}

pub async fn voice_mode_switch(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    mode: &str, host_pseudonym: Option<&str>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::VoiceModeSwitch {
            channel_id: channel_id.into(), mode: mode.into(),
            host_pseudonym: host_pseudonym.map(String::from),
        }).await
}

pub async fn voice_mute(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    target: &str, muted: bool, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::VoiceMute {
            channel_id: channel_id.into(), target_pseudonym: target.into(), muted,
        }).await
}

pub async fn voice_deafen(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    target: &str, deafened: bool, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::VoiceDeafen {
            channel_id: channel_id.into(), target_pseudonym: target.into(), deafened,
        }).await
}

pub async fn voice_roster(
    node: &TransportNode, meshes: &RwLock<MeshMap>,
    community_id: &str, sender: &str, channel_id: &str,
    participants: Vec<VoiceParticipant>, signing_key: &[u8; 32],
) -> BroadcastReport {
    control(node, meshes, community_id, sender, signing_key,
        ControlPayload::VoiceRoster { channel_id: channel_id.into(), participants }).await
}
