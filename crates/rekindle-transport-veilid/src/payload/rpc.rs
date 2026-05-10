//! RPC serialization/deserialization for app_call operations.
//!
//! Types live in `rekindle-types::rpc_payload`. This module provides
//! the transport-specific postcard serialization and TypeId-based dispatch.

pub use rekindle_types::rpc_payload::*;

use crate::error::{TransportError, Result};
use crate::frame::TypeId;

/// Deserialize an inbound app_call payload by TypeId.
pub fn deserialize_inbound_call(type_id: TypeId, bytes: &[u8]) -> Result<InboundCall> {
    match type_id {
        TypeId::CommunityLeave => {
            let req: CommunityLeaveNotification = postcard::from_bytes(bytes)
                .map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8, reason: e.to_string(),
                })?;
            Ok(InboundCall::CommunityLeave(req))
        }
        TypeId::CommunityGovOp => {
            let req: GovernanceRequest = postcard::from_bytes(bytes)
                .map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8, reason: e.to_string(),
                })?;
            Ok(InboundCall::CommunityGovOp(req))
        }
        TypeId::SyncRequest => {
            let req: SyncRequest = postcard::from_bytes(bytes)
                .map_err(|e| TransportError::DeserializationFailed {
                    type_id: type_id as u8, reason: e.to_string(),
                })?;
            Ok(InboundCall::Sync(req))
        }
        TypeId::DmCall => Ok(InboundCall::Dm(bytes.to_vec())),
        _ => Err(TransportError::UnknownType { type_id: type_id as u8 }),
    }
}

/// Serialize a CallResponse for transmission back via app_call_reply.
pub fn serialize_call_response(response: &CallResponse) -> Vec<u8> {
    postcard::to_stdvec(response).unwrap_or_else(|_| b"NAK".to_vec())
}
