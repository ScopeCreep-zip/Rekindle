//! The GUI's session signals (plan E3.4 S8-pre): its voice engine's
//! channel, its focused channel and sharing policy, and the voice roster
//! reconcile over its voice signaling. The Host's beat path reads them
//! through `ShellHooks::session_presence`, and the registry poll through
//! `CommunityPresenceDeps::session_presence`, until E4.1 makes voice the
//! Host's.

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

pub(crate) struct DesktopSessionPresence {
    state: Arc<AppState>,
}

impl DesktopSessionPresence {
    pub(crate) fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

/// We joined or left a voice channel in `community_id`: the Host writes
/// our beat at once and hands the voice roster what the members' beats
/// show (plan E3.4 "Constant rate"), so who is already in the channel is
/// found without waiting on a poll.
pub(crate) fn voice_presence_changed(state: &Arc<AppState>, community_id: &str) {
    let Some(host) = state.host.current() else {
        return;
    };
    let community_id = community_id.to_owned();
    state_helpers::login_scope_or_closed(state).spawn_or_drop("voice presence beat", async move {
        host.voice_presence_changed(&community_id).await;
    });
}

impl rekindle_presence::SessionPresence for DesktopSessionPresence {
    fn self_session(&self, community_id: &str) -> rekindle_types::presence::MemberSession {
        // The identity's status is the session vocabulary (one language
        // for friends and community presence).
        let status = state_helpers::identity_status(&self.state).unwrap_or_default();
        let location = {
            let communities = self.state.communities.read();
            communities
                .get(community_id)
                .and_then(|c| c.my_session_location.clone())
        };
        rekindle_types::presence::MemberSession {
            status,
            location,
            // Activity (game string) is policy-gated and wired in a
            // follow-up; the session always carries a fresh last_active
            // so peers can bucket our last-seen.
            activity: None,
            last_active: rekindle_utils::timestamp_secs(),
        }
    }

    fn presence_policy(
        &self,
        community_id: &str,
    ) -> rekindle_types::presence::PresenceSharingPolicy {
        let communities = self.state.communities.read();
        communities
            .get(community_id)
            .map(|c| c.presence_policy.clone())
            .unwrap_or_default()
    }

    fn active_voice_channel(&self, community_id: &str) -> Option<String> {
        let ve = self.state.voice_engine.lock();
        ve.as_ref()
            .filter(|h| h.community_id.as_deref() == Some(community_id))
            .map(|h| h.channel_id.clone())
    }

    fn reconcile_voice_roster(
        &self,
        community_id: &str,
        rows: Vec<rekindle_presence::VoicePresenceRow>,
    ) {
        // Cheap pre-gate before building the signaling deps: skip
        // entirely when no voice session is bound to this community.
        if self.active_voice_channel(community_id).is_none() {
            return;
        }
        let Some(app) = self.state.app_handle.read().clone() else {
            return;
        };
        let Ok(deps) =
            crate::services::voice_runtime::build_voice_signaling_deps(&app, &self.state)
        else {
            return;
        };
        let views: Vec<rekindle_voice::signaling::PresencePeerView> = rows
            .into_iter()
            .map(|r| rekindle_voice::signaling::PresencePeerView {
                pseudonym_hex: r.pseudonym_hex,
                display_name: r.display_name,
                voice_channel_id: r.voice_channel_id,
                fresh: r.fresh,
            })
            .collect();
        let cid = community_id.to_string();
        crate::state_helpers::login_scope_or_closed(&self.state).spawn_or_drop(
            "voice roster reconcile",
            async move {
                rekindle_voice::signaling::reconcile_from_presence(&deps, &cid, views).await;
            },
        );
    }
}
