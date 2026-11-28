//! Voice policy for a bare media-key call admitted by the transport.

use std::sync::Arc;

use rekindle_codec::community::envelope::ControlPayload;

use super::deps::VoiceSignalingDeps;
use super::media_keys::{handle_voice_media_key, handle_voice_media_key_request};

/// The disposition of a media-key call.
#[derive(Debug)]
pub enum MediaKeyAnswer {
    /// A key was installed. Carries its acknowledgement payload.
    Installed(Box<ControlPayload>),
    /// A valid request was accepted and deliveries were scheduled.
    Requested,
    /// The receiver refused the call.
    Refused,
}

/// Answer with the session's disposition. The transport encodes the reply.
pub async fn answer_media_key_call(
    deps: &Arc<dyn VoiceSignalingDeps>,
    payload: &ControlPayload,
) -> MediaKeyAnswer {
    match payload {
        ControlPayload::VoiceMediaKey { .. } => handle_voice_media_key(deps, payload)
            .await
            .map_or(MediaKeyAnswer::Refused, |ack| {
                MediaKeyAnswer::Installed(Box::new(ack))
            }),
        ControlPayload::VoiceMediaKeyRequest { .. } => {
            if handle_voice_media_key_request(deps, payload).await {
                MediaKeyAnswer::Requested
            } else {
                MediaKeyAnswer::Refused
            }
        }
        _ => MediaKeyAnswer::Refused,
    }
}
