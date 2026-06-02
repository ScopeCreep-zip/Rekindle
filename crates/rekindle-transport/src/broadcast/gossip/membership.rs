//! Member lifecycle, onboarding, role, and join-decision broadcasts.

use parking_lot::RwLock;

use super::super::node::TransportNode;
use super::super::send::BroadcastReport;
use super::{build_sign_send, control, MeshMap};
use crate::payload::gossip::{ControlPayload, GossipPayload, OnboardingAnswer};

pub async fn member_join_request(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    invite_code: Option<&str>,
    route_blob: Option<Vec<u8>>,
    prekey_bundle: Option<Vec<u8>>,
    claimed_subkey_index: Option<u32>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberJoinRequest {
        pseudonym_key: pseudonym_key.into(),
        display_name: display_name.into(),
        invite_code: invite_code.map(String::from),
        route_blob,
        prekey_bundle,
        claimed_subkey_index,
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        pseudonym_key,
        signing_key,
        payload,
    )
    .await
}

pub async fn member_leave(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    pseudonym_key: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberLeave {
        pseudonym_key: pseudonym_key.into(),
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        pseudonym_key,
        signing_key,
        payload,
    )
    .await
}

pub async fn member_joined(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender_pseudonym: &str,
    pseudonym_key: &str,
    display_name: &str,
    role_ids: Vec<u32>,
    status: &str,
    route_blob: Option<Vec<u8>>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberJoined {
        pseudonym_key: pseudonym_key.into(),
        display_name: display_name.into(),
        role_ids,
        status: status.into(),
        route_blob,
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        sender_pseudonym,
        signing_key,
        payload,
    )
    .await
}

pub async fn member_removed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberRemoved {
        pseudonym_key: target_pseudonym.into(),
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        sender_pseudonym,
        signing_key,
        payload,
    )
    .await
}

pub async fn member_roles_changed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    pseudonym: &str,
    role_ids: Vec<u32>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MemberRolesChanged {
            pseudonym_key: pseudonym.into(),
            role_ids,
        },
    )
    .await
}

pub async fn onboarding_complete(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    pseudonym: &str,
    role_ids: Vec<u32>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::OnboardingComplete {
            pseudonym_key: pseudonym.into(),
            role_ids,
        },
    )
    .await
}

pub async fn submit_onboarding_answers(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    answers: Vec<OnboardingAnswer>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SubmitOnboardingAnswers { answers },
    )
    .await
}

pub async fn channel_overwrite_changed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ChannelOverwriteChanged {
            channel_id: channel_id.into(),
        },
    )
    .await
}

pub async fn join_accepted(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    mek_encrypted: Vec<u8>,
    mek_generation: u64,
    registry_key: Option<&str>,
    slot_index: Option<u32>,
    wrapped_slot_seed: Option<Vec<u8>>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::JoinAccepted {
            mek_encrypted,
            mek_generation,
            member_registry_key: registry_key.map(String::from),
            slot_index,
            wrapped_slot_seed,
        },
    )
    .await
}

pub async fn join_rejected(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    reason: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::JoinRejected {
            reason: reason.into(),
        },
    )
    .await
}
