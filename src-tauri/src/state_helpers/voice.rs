//! Read the running voice session from the Host.

use rekindle_voice::session_state::VoiceSessionState;
use rekindle_voice::transport::roster::MediaRoster;
use std::sync::Arc;

use crate::state::AppState;

pub fn voice_media(state: &AppState) -> Option<Arc<MediaRoster>> {
    state.host.current()?.voice().media()
}

pub(crate) fn voice_session_state_for(
    state: &AppState,
    community: &str,
    channel: &str,
) -> Option<Arc<VoiceSessionState>> {
    state
        .host
        .current()?
        .voice()
        .channel_state(community, channel)
}
