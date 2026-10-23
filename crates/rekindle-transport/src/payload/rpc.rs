//! RPC (request/response) payload types for app_call operations.
//!
//! Every RPC payload is wrapped in a [`SignedPayload`] for authentication
//! before transmission. RPC is used ONLY for real-time operations where
//! both parties must be online:
//!
//! - **Governance operations** — admin commands from non-operator nodes
//! - **Leave notification** — best-effort cleanup + rekey trigger
//! - **Sync requests** — history sync to archiver nodes
//!
//! Community join, friend requests, DMs, and all other critical lifecycle
//! state changes use DHT (fully async, offline-safe). See
//! `operations/community.rs`, `operations/friend.rs`, `operations/dm.rs`.

use serde::{Deserialize, Serialize};

use crate::error::{Result, TransportError};
use crate::frame::TypeId;

// ── Community join is DHT-based ──────────────────────────────────────────
//
// Join requests are written to the community's join inbox DHT record
// (DFLT(32) with published keypair). The owner's daemon polls the inbox
// and processes requests asynchronously. See `operations/community.rs`.
//
// The old CommunityJoinRequest/CommunityJoinResponse RPC types have been
// removed — they were synchronous and required both parties online.

/// A single MEK wrapped for a specific member via ECDH, with its scope
/// already parsed from the control payload's channel field.
#[derive(Debug, Clone)]
pub struct MekTransferPayload {
    pub scope: rekindle_types::channel_keys::KeyScope,
    pub generation: u64,
    pub rotator_pseudonym_hex: String,
    pub wrapped_mek: Vec<u8>,
}

/// Compact channel entry for join responses (no log_key or per-member data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEntrySummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mek_generation: u64,
}

// ── Community leave ─────────────────────────────────────────────────────

/// Notification sent when a member leaves a community.
///
/// Best-effort and addressed to peers, not to an owner: under v2.0 the
/// leaver retires its own registry slot, so a recipient removes nobody.
/// What the notification buys is speed — every recipient starts the
/// deterministic MEK rotation immediately instead of waiting for the
/// departed member's presence to go stale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityLeaveNotification {
    pub governance_key: String,
    pub leaving_pseudonym_hex: String,
}

// ── Governance operations ───────────────────────────────────────────────

// The `GovernanceOp` RPC mechanism lived here: a member asked the
// community's "operator" over `app_call` to perform a privileged write.
// Every variant is gone and so is the enum — v2.0 has no operator, and
// each operation became a governance entry the requester writes itself
// and every peer validates on merge. The last four (CreateRole,
// UpdateRole, DeleteRole, TransferOwnership) had no sender at all;
// ownership in particular could not mean anything, since
// `GovernanceState.creator` is fixed at genesis.

// ── Legacy types (sync/bootstrap) ───────────────────────────────────────

/// Sync request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub channel_id: String,
    pub since_timestamp: u64,
}

/// Sync response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub channel_id: String,
    pub messages: Vec<Vec<u8>>,
}

// ── Inbound call dispatch ───────────────────────────────────────────────

/// Inbound RPC request dispatched to the handler.
///
/// Only real-time operations that require both parties online use RPC.
/// Community join is DHT-based (see `operations/community.rs`).
#[derive(Debug, Clone)]
pub enum InboundCall {
    /// Member leaving (best-effort notification for cleanup + rekey).
    CommunityLeave(CommunityLeaveNotification),
    /// History sync request from archiver.
    Sync(SyncRequest),
    /// A wrapped MEK delivered by the deterministic rotator after a
    /// member departed (`communities-channels.md`, "Distribution
    /// paths": *delivers the wrapped MEK to each member via `app_call`*).
    ///
    /// Unlike every other variant this one does **not** arrive framed
    /// and signed — it is a bare Cap'n Proto `CommunityEnvelope`, the
    /// format the desktop track sends. It is admitted without an
    /// envelope signature because the payload authenticates its own
    /// sender: `unwrap_mek` feeds the sender's pseudonym public key
    /// into the ECDH, so only the real rotator can produce ciphertext
    /// that decrypts. See `AppCaller::call_community_envelope`.
    ///
    /// Carries **protocol's** `MekTransferPayload`, not the
    /// transport-local one above: that is the type on this wire, and
    /// minting a fourth MEK-transfer struct to restate it is the
    /// drift this branch exists to remove.
    CommunityMekTransfer(rekindle_codec::community::envelope::MekTransferPayload),
    /// A peer asking for the chunks of an attachment we hold
    /// (architecture §28.9), as the bare Cap'n Proto `RequestAttachment`
    /// envelope `rekindle_files::download` sends. Like the MEK transfer it
    /// arrives unframed: what makes that safe is that the answer is chunk
    /// ciphertext under the attachment's FEK, which only a member holding
    /// the community key can open, and the request itself changes nothing.
    RequestAttachment {
        /// The attachment's 16-byte id.
        attachment_id: [u8; 16],
        /// The chunk indices asked for.
        requested_chunks: Vec<u32>,
    },
    /// A joiner's bootstrap request (plan E3.2), bare like the MEK
    /// transfer; its authority is the joiner's signed `MemberJoined`,
    /// which the handler fetches and verifies by `member_joined`. The
    /// handler answers `CallResponse::Ok` with the encoded
    /// `bootstrapResponseV2` envelope; anything else is a NAK.
    BootstrapRequest {
        joiner_pseudonym: String,
        anchor: String,
        member_joined: rekindle_types::governance::OpRef,
    },
}

/// Response from the handler to an inbound RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallResponse {
    /// Successful response with serialized payload.
    Ok(Vec<u8>),
    /// Acknowledged but no data to return.
    Ack,
    /// Request rejected with reason.
    Rejected { reason: String },
}

/// Deserialize an inbound app_call payload by TypeId.
pub fn deserialize_inbound_call(type_id: TypeId, bytes: &[u8]) -> Result<InboundCall> {
    match type_id {
        TypeId::CommunityLeave => {
            let req: CommunityLeaveNotification =
                postcard::from_bytes(bytes).map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8,
                    reason: e.to_string(),
                })?;
            Ok(InboundCall::CommunityLeave(req))
        }
        TypeId::SyncRequest => {
            let req: SyncRequest =
                postcard::from_bytes(bytes).map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8,
                    reason: e.to_string(),
                })?;
            Ok(InboundCall::Sync(req))
        }
        _ => Err(TransportError::UnknownType {
            type_id: type_id as u8,
        }),
    }
}

/// Serialize a CallResponse for transmission back via app_call_reply.
pub fn serialize_call_response(response: &CallResponse) -> Vec<u8> {
    postcard::to_stdvec(response).unwrap_or_else(|_| crate::operations::direct::NAK.to_vec())
}
