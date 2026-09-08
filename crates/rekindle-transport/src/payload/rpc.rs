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

/// A single MEK wrapped for a specific member via ECDH.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MekTransferPayload {
    pub channel_id: String,
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

/// Bootstrap request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRequest {
    pub joiner_pseudonym: String,
    pub governance_key: String,
}

/// Bootstrap response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResponse {
    pub governance_entries: Vec<Vec<u8>>,
    pub member_list: Vec<Vec<u8>>,
    pub channel_meks: Vec<Vec<u8>>,
    pub recent_messages: Vec<Vec<u8>>,
    pub wrapped_owner_keypair: Vec<u8>,
}

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

// ── 1:1 Call invite (W16.5b) ────────────────────────────────────────────

/// Caller's `CallInvite` payload, dispatched via Veilid `app_call`.
///
/// W16.5b — replaces the old `DmPayload::CallInvite` (which travelled as
/// fire-and-forget `app_message`). The hybrid design uses `app_call` for
/// the wire-level invite-and-ringing handshake (5–10 s budget; matches
/// SIP 100-Trying / 180-Ringing semantics), then `app_message` for the
/// post-decision `CallAccept` / `CallDecline` (user-decision time
/// unbounded by Veilid's RPC budget).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInvitePayload {
    /// Hex-encoded 16-byte random call identifier (32 chars).
    pub call_id: String,
    /// 0 = audio, 1 = video. Matches `rekindle_calls::CallKind::as_u8`.
    pub offer_kind: u8,
    /// Initiator's ephemeral X25519 public key (32 bytes). Receiver
    /// derives the shared `call_key` via X25519 ECDH + HKDF-SHA256
    /// once they accept and learn the acceptor's X25519 pub.
    pub initiator_x25519_pub: Vec<u8>,
    /// ms epoch when the ring expires (caller's clock).
    pub expires_at_ms: u64,
}

/// Receiver's synchronous `CallRinging` reply payload, returned inside
/// `app_call_reply`. W16.5b — receiver's state machine processes the
/// invite (persists CallState=Incoming, emits IncomingCall notification,
/// spawns 30s ring timer) and replies with this within ~100 ms, well
/// inside Veilid's 5–10 s RPC budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRingingPayload {
    pub call_id: String,
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
    /// DM-class message via app_call (friend handshake).
    Dm(Vec<u8>),
    /// W16.5b — 1:1 call invite. Receiver replies synchronously with
    /// `CallResponse::CallRinging` via `app_call_reply`.
    CallInvite(CallInvitePayload),
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
    /// that decrypts. See `Caller::call_community_envelope`.
    ///
    /// Carries **protocol's** `MekTransferPayload`, not the
    /// transport-local one above: that is the type on this wire, and
    /// minting a fourth MEK-transfer struct to restate it is the
    /// drift this branch exists to remove.
    CommunityMekTransfer(rekindle_protocol::dht::community::envelope::MekTransferPayload),
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
    /// W16.5b — typed reply for `InboundCall::CallInvite`. Carries
    /// `CallRingingPayload` so the caller can drive its state machine
    /// with `CallEvent::RingingReceived` and emit
    /// `TransportNotification::CallRinging`.
    CallRinging(CallRingingPayload),
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
        TypeId::DmCall => Ok(InboundCall::Dm(bytes.to_vec())),
        TypeId::CallInvite => {
            let req: CallInvitePayload =
                postcard::from_bytes(bytes).map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8,
                    reason: e.to_string(),
                })?;
            Ok(InboundCall::CallInvite(req))
        }
        _ => Err(TransportError::UnknownType {
            type_id: type_id as u8,
        }),
    }
}

/// Serialize a CallResponse for transmission back via app_call_reply.
pub fn serialize_call_response(response: &CallResponse) -> Vec<u8> {
    postcard::to_stdvec(response).unwrap_or_else(|_| b"NAK".to_vec())
}
