//! The voice engine's media-key delivery on the Host outbox.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use rekindle_voice::signaling::{MediaKeyAttempt, MediaKeyOutbox, VoiceSignalingDeps};

use super::deps::HostVoiceDeps;
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::outbox::{MediaKeyCarrier, MediaKeyDelivery, MediaKeyStreams};

pub(super) struct HostMediaKeyOutbox(pub Weak<DaemonContext>);

impl HostMediaKeyOutbox {
    fn media_keys(&self) -> Result<MediaKeyStreams, String> {
        self.0
            .upgrade()
            .and_then(|ctx| ctx.outbox.media_keys())
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
        self.media_keys()?
            .queue(session, recipient, client_id, sealed)
            .await
            .map(drop)
            .map_err(|error| error.to_string())
    }

    async fn discard_session(&self, session: &str) -> Result<usize, String> {
        self.media_keys()?
            .discard_session(session)
            .await
            .map_err(|error| error.to_string())
    }
}

struct VoiceMediaKeyCarrier(Weak<DaemonContext>);

#[async_trait]
impl MediaKeyCarrier for VoiceMediaKeyCarrier {
    async fn deliver(&self, payload: &[u8]) -> MediaKeyDelivery {
        let Some(ctx) = self.0.upgrade() else {
            return MediaKeyDelivery::Superseded;
        };
        let deps: Arc<dyn VoiceSignalingDeps> = Arc::new(HostVoiceDeps::new(ctx, None));
        match rekindle_voice::signaling::attempt_media_key(&deps, payload).await {
            MediaKeyAttempt::Acknowledged => MediaKeyDelivery::Acknowledged,
            MediaKeyAttempt::Unacknowledged => MediaKeyDelivery::Unacknowledged,
            MediaKeyAttempt::Superseded => MediaKeyDelivery::Superseded,
        }
    }
}

/// Install the same carrier on each Host's unlock.
pub(crate) fn install_carrier(ctx: &DaemonContext) {
    ctx.outbox
        .install_media_key_carrier(Arc::new(VoiceMediaKeyCarrier(ctx.this.clone())));
}
