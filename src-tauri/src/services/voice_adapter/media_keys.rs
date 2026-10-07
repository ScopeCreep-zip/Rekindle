//! `impl MediaKeySource for VoiceAdapter`: where the voice session's
//! SFrame keys come from (`rekindle_voice::media_crypto`), and the request
//! a media path sends for a sender key it lacks (plan C7.20).

use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys;
use rekindle_voice::{CallMediaKeys, MediaKeySource};

use super::VoiceAdapter;
use crate::state::AppState;

impl MediaKeySource for VoiceAdapter {
    fn channel_sender_keys(&self, community_id: &str, channel_id: &str) -> Arc<ChannelSenderKeys> {
        self.state.voice_sender_keys.keys(community_id, channel_id)
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

/// Ask `sender` for its media key at `index`: a voice or video frame
/// arrived under a key it has not sent us, or whose push was lost. Sent to
/// the channel roster (ttl = 0); only `sender` answers.
pub(crate) fn request_media_key(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    sender: &str,
    index: u64,
) {
    let request = CommunityEnvelope::Control(ControlPayload::VoiceMediaKeyRequest {
        channel_id: channel_id.to_string(),
        sender: sender.to_string(),
        key_index: index,
    });
    if let Err(error) =
        crate::services::community::send_to_channel_peers(state, community_id, channel_id, &request)
    {
        tracing::debug!(community = %community_id, channel = %channel_id, %error,
            "media key request not sent");
    }
}
