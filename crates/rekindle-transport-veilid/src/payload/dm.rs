//! DM payload serialization/deserialization for app_message operations.
//!
//! Types live in `rekindle-types::dm_payload`. This module provides
//! the transport-specific postcard serialization and TypeId mapping.

pub use rekindle_types::dm_payload::*;

use crate::error::{TransportError, Result};
use crate::frame::TypeId;

/// Deserialize a DM payload from raw bytes based on the frame TypeId.
pub fn deserialize_dm(type_id: TypeId, bytes: &[u8]) -> Result<DmPayload> {
    postcard::from_bytes(bytes).map_err(|e| TransportError::DeserializationFailed {
        type_id: type_id as u8,
        reason: e.to_string(),
    })
}

/// Serialize a DM payload to bytes.
pub fn serialize_dm(payload: &DmPayload) -> Result<Vec<u8>> {
    postcard::to_stdvec(payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

/// Map a DmPayload variant to its frame TypeId.
pub fn dm_type_id(payload: &DmPayload) -> TypeId {
    match payload {
        DmPayload::Typing { .. } => TypeId::DmTyping,
        DmPayload::FriendRequestAck => TypeId::FriendRequestAck,
        DmPayload::Unfriend => TypeId::Unfriend,
        DmPayload::UnfriendAck => TypeId::UnfriendAck,
        DmPayload::ProfileKeyRotated { .. } => TypeId::ProfileKeyRotated,
        DmPayload::PresenceUpdate { .. } => TypeId::DmPresenceUpdate,
    }
}
