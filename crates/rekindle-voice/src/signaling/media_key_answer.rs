//! The answer to an inbound media-key `app_call` (plans C7.22, E4.1; moved
//! from `src-tauri/src/services/voice_signaling_adapter/mod.rs`).
//!
//! An installed `VoiceMediaKey` is answered with its `VoiceMediaKeyAck`
//! envelope, the only reply the sender counts as delivered
//! (`media_key_outbox::acknowledges`); an accepted `VoiceMediaKeyRequest`
//! is answered empty (the keys follow as their own calls); every refusal
//! is the transport's one NAK, which the host writes for
//! [`MediaKeyAnswer::Refused`]. There is no bare `ACK`.

use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};

use super::deps::VoiceSignalingDeps;
use super::media_keys::{handle_voice_media_key, handle_voice_media_key_request};

/// How a media-key call is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaKeyAnswer {
    /// The encoded `VoiceMediaKeyAck` envelope for the installed key.
    Ack(Vec<u8>),
    /// A request we act on: an empty reply.
    Accepted,
    /// Refused: the transport's NAK.
    Refused,
}

/// Answer an inbound media-key call carrying `payload`.
pub async fn answer_media_key_call(
    deps: &Arc<dyn VoiceSignalingDeps>,
    payload: &ControlPayload,
) -> MediaKeyAnswer {
    match payload {
        ControlPayload::VoiceMediaKey { .. } => {
            let ack = handle_voice_media_key(deps, payload).await.and_then(|ack| {
                rekindle_codec::capnp_envelope::encode_community_envelope(
                    &CommunityEnvelope::control(ack),
                )
                .ok()
            });
            match ack {
                Some(bytes) => MediaKeyAnswer::Ack(bytes),
                None => MediaKeyAnswer::Refused,
            }
        }
        ControlPayload::VoiceMediaKeyRequest { .. } => {
            if handle_voice_media_key_request(deps, payload) {
                MediaKeyAnswer::Accepted
            } else {
                MediaKeyAnswer::Refused
            }
        }
        _ => MediaKeyAnswer::Refused,
    }
}
