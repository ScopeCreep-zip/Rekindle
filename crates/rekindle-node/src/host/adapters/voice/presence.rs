//! The Host's held voice session feeds and consumes community beats.

use std::sync::{Arc, Weak};

use rekindle_presence::{SessionPresence, VoicePresenceRow};
use rekindle_types::subscription_events::VoiceScope;
use rekindle_voice::signaling::{PresencePeerView, VoiceSignalingDeps};

use crate::daemon::dispatch::DaemonContext;

use super::deps::HostVoiceDeps;

/// The Host's projection as a voice-presence port.
#[derive(Default)]
pub struct HostVoicePresence(pub(crate) Weak<DaemonContext>);

impl crate::host::Host {
    /// Share the same voice presence port with the current registry reader.
    #[must_use]
    pub fn voice_presence(&self) -> Arc<dyn SessionPresence> {
        Arc::new(HostVoicePresence(Arc::downgrade(&self.ctx)))
    }
}

impl SessionPresence for HostVoicePresence {
    fn active_voice_channel(&self, community: &str) -> Option<String> {
        match self.0.upgrade()?.voice.session()? {
            VoiceScope::Community {
                community: cid,
                channel,
            } if cid == community => Some(channel),
            _ => None,
        }
    }

    fn reconcile_voice_roster(&self, community: &str, rows: Vec<VoicePresenceRow>) {
        if self.active_voice_channel(community).is_none() {
            return;
        }
        let Some(ctx) = self.0.upgrade() else { return };
        let scope = ctx.unlock_scope_or_closed();
        let deps: Arc<dyn VoiceSignalingDeps> = Arc::new(HostVoiceDeps::new(ctx, None));
        let views = rows
            .into_iter()
            .map(|row| PresencePeerView {
                pseudonym_hex: row.pseudonym_hex,
                voice_channel_id: row.voice_channel_id,
                fresh: row.fresh,
            })
            .collect();
        let community = community.to_owned();
        scope.spawn_or_drop("voice roster reconcile", async move {
            rekindle_voice::signaling::reconcile_from_presence(&deps, &community, views).await;
        });
    }
}

#[cfg(feature = "netsim-entry")]
impl crate::host::Host {
    /// Observe the voice claims consumed by the beat-to-roster path.
    pub fn netsim_voice_claims(&self, community: &str) -> Vec<VoicePresenceRow> {
        self.ctx
            .communities
            .map()
            .read()
            .get(community)
            .map_or_else(Vec::new, |state| state.beat_view.voice_rows())
    }
}
