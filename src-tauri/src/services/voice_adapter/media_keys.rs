//! `impl MediaKeySource for VoiceAdapter`: where the voice session's
//! SFrame keys come from (`rekindle_voice::media_crypto`).

use std::sync::Arc;

use rekindle_secrets::sframe::SframeSender;
use rekindle_voice::{CallMediaKeys, MediaKeySource};

use super::VoiceAdapter;

impl MediaKeySource for VoiceAdapter {
    fn keys(&self) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        crate::state_helpers::key_provider(&self.state)
    }

    fn channel_media_sender(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Arc<SframeSender> {
        self.state
            .voice_media_senders
            .sender_for(community_id, channel_id, generation)
    }

    fn call_media(&self, peer_pubkey: &str) -> Option<CallMediaKeys> {
        let call = self
            .state
            .active_calls
            .list_all()
            .into_iter()
            .find(|c| c.peer_pubkey == peer_pubkey)?;
        Some(CallMediaKeys {
            secret: call.media_secret?,
            sender: call.media_sender,
        })
    }
}
