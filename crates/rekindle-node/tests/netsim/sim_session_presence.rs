//! A shell's session signals in the netsim (plan E3.4 S8-pre): the voice
//! channel a scenario puts the member in, and every voice roster the Host
//! hands the shell, recorded.

use std::collections::BTreeMap;

use parking_lot::Mutex;
use rekindle_presence::VoicePresenceRow;
use rekindle_types::presence::{MemberSession, PresenceSharingPolicy};

#[derive(Default)]
pub struct SimSessionPresence {
    /// The voice channel we are in, by community.
    pub voice: Mutex<BTreeMap<String, String>>,
    /// Every roster handed to the voice layer: community, rows.
    pub rosters: Mutex<Vec<(String, Vec<VoicePresenceRow>)>>,
}

impl rekindle_presence::SessionPresence for SimSessionPresence {
    fn active_voice_channel(&self, community_id: &str) -> Option<String> {
        self.voice.lock().get(community_id).cloned()
    }

    fn reconcile_voice_roster(&self, community_id: &str, rows: Vec<VoicePresenceRow>) {
        self.rosters.lock().push((community_id.to_owned(), rows));
    }

    fn self_session(&self, _community_id: &str) -> MemberSession {
        MemberSession::default()
    }

    fn presence_policy(&self, _community_id: &str) -> PresenceSharingPolicy {
        PresenceSharingPolicy::default()
    }
}
