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
use crate::payload::rpc::{CallResponse, CommunityLeaveNotification, SyncRequest, SyncResponse};

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
