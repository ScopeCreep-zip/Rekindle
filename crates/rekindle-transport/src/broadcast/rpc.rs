//! Outbound RPC calls — request-response via Veilid app_call.
//!
//! RPC is used ONLY for operations requiring both parties online:
//! - Community leave notification (best-effort cleanup + rekey trigger)
//! - Governance operations (admin commands from non-operator nodes)
//! - Sync requests/responses (history sync to archiver nodes)
//!
//! All persistent lifecycle operations (join, friend request, DMs) use
//! DHT writes instead — see `dm.rs` and `dht_writes.rs`.

use std::time::Duration;

use tracing::{debug, info};

use super::node::TransportNode;
use super::peer_registry::PeerTarget;
use crate::error::{Result, TransportError};
use crate::frame::TypeId;
use crate::payload::rpc::{
    CallResponse, CommunityLeaveNotification, GovernanceOp, GovernanceOpResponse,
    GovernanceRequest, SyncRequest, SyncResponse,
};

/// Extended timeout for sync operations that may transfer large payloads.
const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

// ── Community leave ────────────────────────────────────────────────────

/// Send a community leave notification via RPC to the community route.
///
/// Best-effort — if the owner is offline, cleanup happens when they
/// come back and poll the join inbox (which has a Leave entry from DHT).
pub async fn community_leave(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    leaving_pseudonym: &str,
    signing_key: &[u8; 32],
    sender_public_hex: &str,
) -> Result<CallResponse> {
    debug!(
        governance = governance_key,
        pseudonym = leaving_pseudonym,
        "rpc: community_leave"
    );
    let notification = CommunityLeaveNotification {
        governance_key: governance_key.into(),
        leaving_pseudonym_hex: leaving_pseudonym.into(),
    };
    let payload =
        postcard::to_stdvec(&notification).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;

    let response_bytes = node
        .caller()
        .call(
            target,
            TypeId::CommunityLeave,
            signing_key,
            sender_public_hex,
            &payload,
        )
        .await?;

    let response: CallResponse = postcard::from_bytes(&response_bytes).unwrap_or(CallResponse::Ack);

    info!(governance = governance_key, "community leave RPC complete");
    Ok(response)
}

// ── Governance operations ──────────────────────────────────────────────

/// Send a governance operation to the community owner's daemon.
///
/// The operation is validated against the sender's roles on the
/// receiving end. Permission denied responses are typed.
pub async fn governance_op(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    operation: GovernanceOp,
    signing_key: &[u8; 32],
    sender_public_hex: &str,
) -> Result<GovernanceOpResponse> {
    debug!(governance = governance_key, op = ?std::mem::discriminant(&operation), "rpc: governance_op");
    let request = GovernanceRequest {
        governance_key: governance_key.into(),
        operation,
    };
    let payload =
        postcard::to_stdvec(&request).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;

    let response_bytes = node
        .caller()
        .call(
            target,
            TypeId::CommunityGovOp,
            signing_key,
            sender_public_hex,
            &payload,
        )
        .await?;

    let response: GovernanceOpResponse =
        postcard::from_bytes(&response_bytes).unwrap_or(GovernanceOpResponse::Failed {
            reason: "response deserialization failed".into(),
        });

    debug!(governance = governance_key, "governance RPC complete");
    Ok(response)
}

/// Convenience: send a Kick governance op.
pub async fn governance_kick(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    target_pseudonym: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::Kick {
            target_pseudonym: target_pseudonym.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a Ban governance op.
pub async fn governance_ban(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    target_pseudonym: &str,
    reason: Option<&str>,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::Ban {
            target_pseudonym: target_pseudonym.into(),
            reason: reason.map(String::from),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send an Unban governance op.
pub async fn governance_unban(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    target_pseudonym: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::Unban {
            target_pseudonym: target_pseudonym.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a Timeout governance op.
pub async fn governance_timeout(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    target_pseudonym: &str,
    duration_secs: u64,
    reason: Option<&str>,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::Timeout {
            target_pseudonym: target_pseudonym.into(),
            duration_seconds: duration_secs,
            reason: reason.map(String::from),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a CreateChannel governance op.
pub async fn governance_create_channel(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    name: &str,
    kind: &str,
    topic: Option<&str>,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::CreateChannel {
            name: name.into(),
            kind: kind.into(),
            topic: topic.map(String::from),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a DeleteChannel governance op.
pub async fn governance_delete_channel(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    channel_id: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::DeleteChannel {
            channel_id: channel_id.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send an UpdateChannel governance op.
pub async fn governance_update_channel(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    channel_id: &str,
    name: Option<&str>,
    topic: Option<&str>,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::UpdateChannel {
            channel_id: channel_id.into(),
            name: name.map(String::from),
            topic: topic.map(String::from),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a CreateRole governance op.
pub async fn governance_create_role(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    name: &str,
    permissions: u64,
    color: u32,
    position: i32,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::CreateRole {
            name: name.into(),
            permissions,
            color,
            position,
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send an UpdateRole governance op.
pub async fn governance_update_role(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    role_id: u32,
    name: Option<&str>,
    permissions: Option<u64>,
    color: Option<u32>,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::UpdateRole {
            role_id,
            name: name.map(String::from),
            permissions,
            color,
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a DeleteRole governance op.
pub async fn governance_delete_role(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    role_id: u32,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::DeleteRole { role_id },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send an AssignRole governance op.
pub async fn governance_assign_role(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    member_pseudonym: &str,
    role_id: u32,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::AssignRole {
            member_pseudonym: member_pseudonym.into(),
            role_id,
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send an UnassignRole governance op.
pub async fn governance_unassign_role(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    member_pseudonym: &str,
    role_id: u32,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::UnassignRole {
            member_pseudonym: member_pseudonym.into(),
            role_id,
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a RotateMek governance op.
pub async fn governance_rotate_mek(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    channel_id: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::RotateMek {
            channel_id: channel_id.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: send a TransferOwnership governance op.
pub async fn governance_transfer_ownership(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    new_owner_pseudonym: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::TransferOwnership {
            new_owner_pseudonym: new_owner_pseudonym.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: register a channel record key via governance RPC.
pub async fn governance_register_channel_record(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    member_pseudonym: &str,
    channel_id: &str,
    record_key: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::RegisterChannelRecord {
            member_pseudonym: member_pseudonym.into(),
            channel_id: channel_id.into(),
            record_key: record_key.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: approve a join request via governance RPC.
pub async fn governance_approve_join(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    target_pseudonym: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::ApproveJoin {
            target_pseudonym: target_pseudonym.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

/// Convenience: reject a join request via governance RPC.
pub async fn governance_reject_join(
    node: &TransportNode,
    target: &PeerTarget,
    governance_key: &str,
    target_pseudonym: &str,
    reason: &str,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<GovernanceOpResponse> {
    governance_op(
        node,
        target,
        governance_key,
        GovernanceOp::RejectJoin {
            target_pseudonym: target_pseudonym.into(),
            reason: reason.into(),
        },
        signing_key,
        sender_hex,
    )
    .await
}

// ── Sync ───────────────────────────────────────────────────────────────

/// Send a sync request to an archiver node.
pub async fn sync_request(
    node: &TransportNode,
    target: &PeerTarget,
    channel_id: &str,
    since_timestamp: u64,
    signing_key: &[u8; 32],
    sender_hex: &str,
) -> Result<SyncResponse> {
    debug!(
        channel = channel_id,
        since = since_timestamp,
        "rpc: sync_request"
    );
    let request = SyncRequest {
        channel_id: channel_id.into(),
        since_timestamp,
    };
    let payload =
        postcard::to_stdvec(&request).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;

    let response_bytes = node
        .caller()
        .call_with_timeout(
            target,
            TypeId::SyncRequest,
            signing_key,
            sender_hex,
            &payload,
            SYNC_TIMEOUT,
        )
        .await?;

    let response: SyncResponse = postcard::from_bytes(&response_bytes).map_err(|e| {
        TransportError::DeserializationFailed {
            type_id: TypeId::SyncResponse as u8,
            reason: e.to_string(),
        }
    })?;

    info!(
        channel = channel_id,
        messages = response.messages.len(),
        "sync response received"
    );
    Ok(response)
}
