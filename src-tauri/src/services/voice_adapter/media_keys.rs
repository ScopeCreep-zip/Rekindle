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
/// arrived under a key it has not delivered to us. A direct `app_call` to
/// the sender's roster route (plan C7.22), never gossip, whose content
/// dedup would drop every retry; the sender answers by delivering its keys.
pub(crate) fn request_media_key(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    sender: &str,
    index: u64,
) {
    let Some(requester) = crate::state_helpers::my_pseudonym_key(state, community_id) else {
        return;
    };
    let Some(transport) =
        crate::state_helpers::voice_transport_for(state, community_id, channel_id)
    else {
        return;
    };
    let request = CommunityEnvelope::Control(ControlPayload::VoiceMediaKeyRequest {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        requester,
        sender: sender.to_string(),
        key_index: index,
    });
    let Ok(bytes) = rekindle_codec::capnp_envelope::encode_community_envelope(&request) else {
        return;
    };
    let state_task = Arc::clone(state);
    let (cid, ch, sender) = (
        community_id.to_string(),
        channel_id.to_string(),
        sender.to_string(),
    );
    crate::state_helpers::login_scope_or_closed(state).spawn_or_drop(
        "voice media key request",
        async move {
            let route = transport
                .lock()
                .await
                .peer_entries()
                .into_iter()
                .find_map(|(peer, route)| (peer == sender).then_some(route));
            let Some(route) = route else {
                tracing::debug!(community = %cid, channel = %ch, %sender,
                    "media key request not sent: sender is not on our roster");
                return;
            };
            if let Err(error) =
                crate::state_helpers::call_route_blob(&state_task, &route, bytes).await
            {
                tracing::debug!(community = %cid, channel = %ch, %sender, %error,
                    "media key request not delivered");
            }
        },
    );
}
