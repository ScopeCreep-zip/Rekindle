//! The fixture's session signals (plan E3.4 S8-pre): the voice channel
//! the test sets, and every roster reconcile recorded.

use std::sync::Arc;

use parking_lot::Mutex;

use super::MockState;

pub struct MockSessionPresence {
    pub state: Arc<Mutex<MockState>>,
}

impl crate::deps::SessionPresence for MockSessionPresence {
    fn self_session(&self, _community_id: &str) -> rekindle_types::presence::MemberSession {
        rekindle_types::presence::MemberSession::default()
    }
    fn presence_policy(
        &self,
        _community_id: &str,
    ) -> rekindle_types::presence::PresenceSharingPolicy {
        rekindle_types::presence::PresenceSharingPolicy::default()
    }
    fn active_voice_channel(&self, _community_id: &str) -> Option<String> {
        self.state.lock().active_voice_channel.clone()
    }
    fn reconcile_voice_roster(&self, community_id: &str, rows: Vec<crate::deps::VoicePresenceRow>) {
        self.state
            .lock()
            .calls_reconcile_voice_roster
            .push((community_id.to_string(), rows));
    }
}
