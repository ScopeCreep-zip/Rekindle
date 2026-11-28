//! Outbound RPC calls — request-response over app calls.
//!
//! RPC is used ONLY for operations requiring both parties online:
//! - Community leave notification (best-effort cleanup + rekey trigger)
//! - Governance operations (admin commands from non-operator nodes)
//! - Sync requests/responses (history sync to archiver nodes)
//!
//! All persistent lifecycle operations (join, friend request, DMs) use
//! DHT writes instead — see `dm.rs` and `dht_writes.rs`.

use tracing::{debug, info};

use super::node::TransportNode;
use crate::error::{Result, TransportError};
use crate::frame::TypeId;
use crate::payload::rpc::{CallResponse, CommunityLeaveNotification};
use rekindle_records::port::PeerTarget;

// ── Community leave ────────────────────────────────────────────────────

/// Send a community leave notification via RPC to the community route.
///
/// Best-effort — if the owner is offline, cleanup happens when they
/// come back and poll the join inbox (which has a Leave entry from DHT).
/// `recipient` is the owner's identity key; the request is signed to it.
pub async fn community_leave(
    node: &TransportNode,
    target: &PeerTarget,
    recipient: &[u8; 32],
    anchor: &str,
    leaving_pseudonym: &str,
    signing_key: &[u8; 32],
    sender_public_hex: &str,
    now_ms: u64,
) -> Result<CallResponse> {
    debug!(
        governance = anchor,
        pseudonym = leaving_pseudonym,
        "rpc: community_leave"
    );
    let notification = CommunityLeaveNotification {
        anchor: anchor.into(),
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
            recipient,
            TypeId::CommunityLeave,
            signing_key,
            sender_public_hex,
            &payload,
            now_ms,
        )
        .await?;

    // An undecodable reply (a NAK among them) is an error, never an Ack.
    let response: CallResponse = postcard::from_bytes(&response_bytes).map_err(|e| {
        TransportError::DeserializationFailed {
            type_id: TypeId::CommunityLeave as u8,
            reason: e.to_string(),
        }
    })?;

    info!(governance = anchor, "community leave RPC complete");
    Ok(response)
}

// ── Governance operations ──────────────────────────────────────────────

// ── Sync ───────────────────────────────────────────────────────────────
