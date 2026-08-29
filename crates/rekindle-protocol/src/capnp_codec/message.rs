
use super::{capnp_err, pack, text_to_string, unpack, ProtocolError};
use crate::message_capnp;
use crate::messaging::envelope::MessageEnvelope;

/// Encode a `MessageEnvelope` into packed Cap'n Proto bytes.
pub fn encode_envelope(env: &MessageEnvelope) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<message_capnp::message_envelope::Builder<'_>>();
        root.set_sender_key(&env.sender_key);
        root.set_timestamp(env.timestamp);
        root.set_nonce(&env.nonce);
        root.set_payload(&env.payload);
        root.set_signature(&env.signature);
    }
    pack(&builder)
}

/// Decode packed Cap'n Proto bytes into a `MessageEnvelope`.
pub fn decode_envelope(data: &[u8]) -> Result<MessageEnvelope, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<message_capnp::message_envelope::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    Ok(MessageEnvelope {
        sender_key: root.get_sender_key().map_err(|e| capnp_err(&e))?.to_vec(),
        timestamp: root.get_timestamp(),
        nonce: root.get_nonce().map_err(|e| capnp_err(&e))?.to_vec(),
        payload: root.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        signature: root.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

/// Encode a chat message body + optional reply-to nonce.
pub fn encode_chat_message(body: &str, reply_to: Option<&[u8]>) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<message_capnp::chat_message::Builder<'_>>();
        root.set_body(body);
        if let Some(rt) = reply_to {
            root.set_reply_to(rt);
        }
        // attachments left empty for now
    }
    pack(&builder)
}

/// Decode packed bytes into (body, optional `reply_to` nonce).
pub fn decode_chat_message(data: &[u8]) -> Result<(String, Option<Vec<u8>>), ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<message_capnp::chat_message::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    let body = text_to_string(root.get_body().map_err(|e| capnp_err(&e))?)?;
    let reply_to = if root.has_reply_to() {
        Some(root.get_reply_to().map_err(|e| capnp_err(&e))?.to_vec())
    } else {
        None
    };

    Ok((body, reply_to))
}

/// Encode a `GameInfo` into packed Cap'n Proto presence `GameStatus` bytes.
///
/// Re-uses the presence schema's `GameStatus` since it's the same structure.
pub fn encode_game_info(info: &crate::messaging::envelope::GameInfo) -> Vec<u8> {
    super::presence::encode_game_status(info)
}

/// Decode packed bytes into a `GameInfo`.
pub fn decode_game_info(
    data: &[u8],
) -> Result<crate::messaging::envelope::GameInfo, ProtocolError> {
    super::presence::decode_game_status(data)
}
