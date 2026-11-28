//! Scoped reads and controls used by the call interpreter and video shell.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys;
use rekindle_types::subscription_events::VoiceScope;
use rekindle_voice::liveness::MEDIA_LIVE_WINDOW_MS;
use rekindle_voice::session_state::VoiceSessionState;
use rekindle_voice::transport::roster::MediaRoster;
use rekindle_voice::transport::VoiceTransport;

use super::{HostVoice, LiveVoice};

impl HostVoice {
    pub(super) fn live_peers(&self) -> HashSet<String> {
        self.live
            .lock()
            .as_ref()
            .map_or_else(HashSet::new, |session| {
                session
                    .liveness
                    .live_within(MEDIA_LIVE_WINDOW_MS, self.clock.now_ms())
            })
    }

    /// The most recent inbound media for the call currently claiming audio.
    #[must_use]
    pub fn direct_media_last_seen(&self, call_id: &str) -> Option<u64> {
        let call = self.call.lock();
        call.as_ref().filter(|call| call.id == call_id)?;
        self.live.lock().as_ref()?.liveness.latest()
    }

    /// Mute toggle used by the current shortcut.
    pub(super) fn toggle_muted(&self) -> Option<bool> {
        let mut session = self.live.lock();
        let live = session.as_mut()?;
        let muted = !live.engine.is_muted;
        live.engine.set_muted(muted);
        live.muted.store(muted, Ordering::Relaxed);
        Some(muted)
    }

    /// The roster shared by media producers and consumers.
    #[must_use]
    pub fn media(&self) -> Option<Arc<MediaRoster>> {
        self.live.lock().as_ref().map(|s| Arc::clone(&s.media))
    }

    /// Subscribe a community video encoder to this call's allocator and lifetime.
    pub fn channel_video_allocation(
        &self,
        community: &str,
        channel: &str,
    ) -> Option<(Arc<MediaRoster>, Arc<rekindle_lifecycle::SessionScope>)> {
        self.channel(community, channel, |s| {
            Some((Arc::clone(&s.media), Arc::clone(s.loops.as_ref()?)))
        })?
    }

    /// The current session's channel media state.
    #[must_use]
    pub fn channel_state(&self, community: &str, channel: &str) -> Option<Arc<VoiceSessionState>> {
        self.channel(community, channel, |s| Arc::clone(&s.session_state))
    }

    /// The channel's own and remote media keys.
    #[must_use]
    pub fn channel_keys(&self, community: &str, channel: &str) -> Arc<ChannelSenderKeys> {
        self.sender_keys.keys(community, channel)
    }

    pub(super) fn channel_transport(
        &self,
        community: &str,
        channel: &str,
    ) -> Option<Arc<tokio::sync::Mutex<VoiceTransport>>> {
        self.channel(community, channel, |s| Arc::clone(&s.transport))
    }

    pub(super) fn channel_media(&self, community: &str, channel: &str) -> Option<Arc<MediaRoster>> {
        self.channel(community, channel, |s| Arc::clone(&s.media))
    }

    fn channel<T>(
        &self,
        community: &str,
        channel: &str,
        read: impl FnOnce(&LiveVoice) -> T,
    ) -> Option<T> {
        let live = self.live.lock();
        let live = live.as_ref()?;
        match &live.scope {
            VoiceScope::Community {
                community: cid,
                channel: chid,
            } if cid == community && chid == channel => Some(read(live)),
            _ => None,
        }
    }
}
