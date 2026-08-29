
use super::{capnp_err, not_in_schema, pack, text_to_string, unpack, ProtocolError};
use crate::voice_capnp;

/// Voice signaling types matching the capnp enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalType {
    Join,
    Leave,
    Offer,
    Answer,
    IceCandidate,
}

/// Domain struct for voice signaling messages.
#[derive(Debug, Clone)]
pub struct VoiceSignaling {
    pub signal_type: SignalType,
    pub channel_id: String,
    pub sender_key: Vec<u8>,
    pub payload: Vec<u8>,
}

pub fn encode_signaling(sig: &VoiceSignaling) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<voice_capnp::voice_signaling::Builder<'_>>();
        let capnp_type = match sig.signal_type {
            SignalType::Join => voice_capnp::voice_signaling::SignalType::Join,
            SignalType::Leave => voice_capnp::voice_signaling::SignalType::Leave,
            SignalType::Offer => voice_capnp::voice_signaling::SignalType::Offer,
            SignalType::Answer => voice_capnp::voice_signaling::SignalType::Answer,
            SignalType::IceCandidate => voice_capnp::voice_signaling::SignalType::IceCandidate,
        };
        root.set_type(capnp_type);
        root.set_channel_id(&sig.channel_id);
        root.set_sender_key(&sig.sender_key);
        root.set_payload(&sig.payload);
    }
    pack(&builder)
}

pub fn decode_signaling(data: &[u8]) -> Result<VoiceSignaling, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<voice_capnp::voice_signaling::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    let signal_type = match root.get_type().map_err(not_in_schema)? {
        voice_capnp::voice_signaling::SignalType::Join => SignalType::Join,
        voice_capnp::voice_signaling::SignalType::Leave => SignalType::Leave,
        voice_capnp::voice_signaling::SignalType::Offer => SignalType::Offer,
        voice_capnp::voice_signaling::SignalType::Answer => SignalType::Answer,
        voice_capnp::voice_signaling::SignalType::IceCandidate => SignalType::IceCandidate,
    };

    Ok(VoiceSignaling {
        signal_type,
        channel_id: text_to_string(root.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        sender_key: root.get_sender_key().map_err(|e| capnp_err(&e))?.to_vec(),
        payload: root.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}
