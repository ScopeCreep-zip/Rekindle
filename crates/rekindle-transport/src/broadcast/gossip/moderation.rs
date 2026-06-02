//! Moderation and system broadcasts (kick/ban/timeout, raid, lockdown).

use parking_lot::RwLock;

use super::super::node::TransportNode;
use super::super::send::BroadcastReport;
use super::{control, MeshMap};
use crate::payload::gossip::ControlPayload;

pub async fn kick(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::Kick {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn ban(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::Ban {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn unban(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::Unban {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn timeout_member(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    duration_seconds: u64,
    reason: Option<&str>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::TimeoutMember {
            target_pseudonym: target.into(),
            duration_seconds,
            reason: reason.map(String::from),
        },
    )
    .await
}

pub async fn remove_timeout(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::RemoveTimeout {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn member_timed_out(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    pseudonym: &str,
    timeout_until: Option<u64>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MemberTimedOut {
            pseudonym_key: pseudonym.into(),
            timeout_until,
        },
    )
    .await
}

pub async fn system_message(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    body: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SystemMessage {
            body: body.into(),
            timestamp: rekindle_utils::timestamp_ms(),
        },
    )
    .await
}

pub async fn raid_alert(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    active: bool,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::RaidAlert { active },
    )
    .await
}

pub async fn channel_lockdown(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    locked: bool,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ChannelLockdown { locked },
    )
    .await
}

pub async fn kicked_notification(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::KickedNotification,
    )
    .await
}
