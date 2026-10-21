//! Call media keys on the Host's outbox `media_key` streams (plan E2.5):
//! the voice signaling queues and discards through the Host's
//! `MediaKeyStreams`, and the Host's `media_key` sender attempts each row through
//! `rekindle_voice::signaling::attempt_media_key`. Forwarding only.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_node::host::adapters::outbox::{MediaKeyCarrier, MediaKeyDelivery, MediaKeyStreams};
use rekindle_voice::signaling::{MediaKeyAttempt, MediaKeyOutbox, VoiceSignalingDeps};

use super::VoiceSignalingAdapter;
use crate::state::AppState;

/// The Host's `media_key` streams as the voice signaling uses them.
pub struct HostMediaKeyOutbox(pub Arc<AppState>);

impl HostMediaKeyOutbox {
    fn media_keys(&self) -> Result<MediaKeyStreams, String> {
        (self.0.host.current())
            .and_then(|host| host.media_keys())
            .ok_or_else(|| "no unlocked outbox".to_string())
    }
}

#[async_trait]
impl MediaKeyOutbox for HostMediaKeyOutbox {
    async fn queue(
        &self,
        session: &str,
        recipient: &str,
        client_id: &str,
        sealed: Vec<u8>,
    ) -> Result<(), String> {
        (self
            .media_keys()?
            .queue(session, recipient, client_id, sealed)
            .await)
            .map_err(|e| e.to_string())
    }

    async fn discard_session(&self, session: &str) -> Result<usize, String> {
        (self.media_keys()?.discard_session(session).await).map_err(|e| e.to_string())
    }
}

/// The voice side of the Host's `media_key` sender.
struct VoiceMediaKeyCarrier(Arc<AppState>);

#[async_trait]
impl MediaKeyCarrier for VoiceMediaKeyCarrier {
    async fn deliver(&self, payload: &[u8]) -> MediaKeyDelivery {
        let Ok(adapter) = crate::services::build_adapter(&self.0, VoiceSignalingAdapter::new)
        else {
            return MediaKeyDelivery::Unacknowledged;
        };
        let deps: Arc<dyn VoiceSignalingDeps> = adapter;
        match rekindle_voice::signaling::attempt_media_key(&deps, payload).await {
            MediaKeyAttempt::Acknowledged => MediaKeyDelivery::Acknowledged,
            MediaKeyAttempt::Unacknowledged => MediaKeyDelivery::Unacknowledged,
            MediaKeyAttempt::Superseded => MediaKeyDelivery::Superseded,
        }
    }
}

/// Install the voice side as the unlocked Host's `media_key` sender.
pub fn install_carrier(state: &Arc<AppState>) {
    if let Some(host) = state.host.current() {
        host.install_media_key_carrier(Arc::new(VoiceMediaKeyCarrier(Arc::clone(state))));
    }
}
