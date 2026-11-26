//! The Host's voice frame sender (plan E4.1 S5a2): each frame leaves on the
//! transport's media-class send (`AppCaller::send_media_to_route`,
//! `MessageClass::Voice`), inside the Veilid boundary (ADR 0014); the
//! route import is the transport's one importer's.

use async_trait::async_trait;
use rekindle_transport::broadcast::send::AppCaller;
use rekindle_voice::{VoiceError, VoiceFrameSender};

/// [`VoiceFrameSender`] over the transport node's app caller.
pub struct HostFrameSender(pub AppCaller);

#[async_trait]
impl VoiceFrameSender for HostFrameSender {
    async fn send_voice_frame(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), VoiceError> {
        self.0
            .send_media_to_route(route_blob, data)
            .await
            .map_err(|e| VoiceError::Transport(e.to_string()))
    }
}
