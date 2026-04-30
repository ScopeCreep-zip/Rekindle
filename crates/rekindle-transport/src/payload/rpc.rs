//! RPC (request/response) payload types for app_call operations.
//!
//! Every RPC payload is wrapped in a [`SignedPayload`] for authentication
//! before transmission. This is the fix for the unsigned app_call vulnerability.

use serde::{Deserialize, Serialize};

use crate::error::{TransportError, Result};
use crate::frame::TypeId;

/// Bootstrap request — sent via app_call when joining a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRequest {
    pub joiner_pseudonym: String,
    pub governance_key: String,
}

/// Bootstrap response — community state for new joiner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResponse {
    pub governance_entries: Vec<Vec<u8>>,
    pub member_list: Vec<Vec<u8>>,
    pub channel_meks: Vec<Vec<u8>>,
    pub recent_messages: Vec<Vec<u8>>,
    pub wrapped_owner_keypair: Vec<u8>,
}

/// MEK transfer request — ECDH-wrapped MEK delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MekTransferRequest {
    pub community_id: String,
    pub channel_id: Option<String>,
    pub generation: u64,
    pub sender_pseudonym: String,
    pub wrapped_mek: Vec<u8>,
}

/// Sync request — request channel history from an archiver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub channel_id: String,
    pub since_timestamp: u64,
}

/// Sync response — channel messages from archiver's local store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub channel_id: String,
    pub messages: Vec<Vec<u8>>,
}

/// Inbound RPC request dispatched to the handler.
#[derive(Debug, Clone)]
pub enum InboundCall {
    Bootstrap(BootstrapRequest),
    MekTransfer(MekTransferRequest),
    Sync(SyncRequest),
    /// DM-class message sent via app_call (friend request/accept handshake).
    Dm(Vec<u8>),
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
        TypeId::BootstrapRequest => {
            let req: BootstrapRequest = postcard::from_bytes(bytes)
                .map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8,
                    reason: e.to_string(),
                })?;
            Ok(InboundCall::Bootstrap(req))
        }
        TypeId::MekTransfer => {
            let req: MekTransferRequest = postcard::from_bytes(bytes)
                .map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8,
                    reason: e.to_string(),
                })?;
            Ok(InboundCall::MekTransfer(req))
        }
        TypeId::SyncRequest => {
            let req: SyncRequest = postcard::from_bytes(bytes)
                .map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8,
                    reason: e.to_string(),
                })?;
            Ok(InboundCall::Sync(req))
        }
        TypeId::DmCall => {
            Ok(InboundCall::Dm(bytes.to_vec()))
        }
        _ => Err(TransportError::UnknownType { type_id: type_id as u8 }),
    }
}

/// Serialize a CallResponse for transmission back via app_call_reply.
pub fn serialize_call_response(response: &CallResponse) -> Vec<u8> {
    postcard::to_stdvec(response).unwrap_or_else(|_| b"NAK".to_vec())
}
