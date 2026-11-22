//! The outbox's `dm` and `group_dm` row payload: a direct body stored as
//! its type id and encoding, framed afresh at each attempt (split from
//! `outbox.rs`; reached as `outbox::payload`).

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};
use serde::{Deserialize, Serialize};

use crate::operations::friend::FriendError;

/// A `dm` row's payload: the body, framed afresh at each attempt.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    type_id: u16,
    #[serde(with = "rekindle_types::base64_bytes")]
    body: Vec<u8>,
}

/// `body` as a `dm` row stores it.
///
/// # Errors
/// The payload could not be serialized.
pub fn payload(body: &DirectBody) -> Result<Vec<u8>, FriendError> {
    serde_json::to_vec(&Payload {
        type_id: body.type_id().as_u16(),
        body: body.encode(),
    })
    .map_err(|e| FriendError::Failed(format!("serialize the outbox payload: {e}")))
}

/// A `dm` or `group_dm` row's payload read back.
///
/// # Errors
/// It is not a stored payload of an allocated kind.
pub(crate) fn decode_payload(payload: &[u8]) -> Result<DirectBody, String> {
    let stored: Payload = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
    let type_id = DmTypeId::from_u16(stored.type_id)
        .ok_or_else(|| format!("typeId {:#06x}", stored.type_id))?;
    DirectBody::decode(type_id, &stored.body).map_err(|e| e.to_string())
}
