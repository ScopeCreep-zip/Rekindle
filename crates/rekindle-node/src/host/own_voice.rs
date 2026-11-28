//! Our voice session on `rekindled` (plan E3.4 S8-pre, the direction E4.1
//! takes: voice is the Host's). One record, one owner: a `VoiceJoin` that
//! succeeds records the session (`operations::voice::join_voice`), and
//! `VoiceLeave`, a leave of or removal from its community, and the lock
//! take it. Our beat says its channel (`voiceChannel`) through
//! [`SessionPresence`], and every join and leave writes that beat at once
//! (`adapters::presence::voice_changed`, the trigger the shell's voice
//! session fires). A shell keeps its own voice session until E4.1
//! (`ShellHooks::session_presence`).

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_presence::{SessionPresence, VoicePresenceRow};
use rekindle_transport::operations::voice::{leave_voice, VoiceSession};

/// The voice channel we are in: one at a time, as in Xfire (`VoiceLeave`
/// names none), so a join elsewhere ends the session it replaces.
#[derive(Clone, Default)]
pub(crate) struct OwnVoice {
    session: Arc<RwLock<Option<VoiceSession>>>,
}

impl OwnVoice {
    /// Record `session`, ending and handing back the one it replaces.
    pub(crate) fn join(&self, session: VoiceSession) -> Option<VoiceSession> {
        let replaced = self.session.write().replace(session);
        replaced.map(end)
    }

    /// End our session, handing it back; `None` when we are in none.
    pub(crate) fn leave(&self) -> Option<VoiceSession> {
        let taken = self.session.write().take();
        taken.map(end)
    }

    /// End our session if it is in `community`, which we are leaving.
    pub(crate) fn leave_community(&self, community: &str) -> Option<VoiceSession> {
        let taken = {
            let mut slot = self.session.write();
            if slot.as_ref()?.community_id == community {
                slot.take()
            } else {
                None
            }
        };
        taken.map(end)
    }
}

fn end(mut session: VoiceSession) -> VoiceSession {
    leave_voice(&mut session);
    session
}

impl SessionPresence for OwnVoice {
    fn active_voice_channel(&self, community_id: &str) -> Option<String> {
        (self.session.read().as_ref())
            .filter(|s| s.community_id == community_id)
            .map(|s| s.channel_id.clone())
    }

    fn reconcile_voice_roster(&self, _community_id: &str, _rows: Vec<VoicePresenceRow>) {
        // `rekindled` runs no media engine until E4.1 (plan E4.1 moves the
        // voice engine into the Host), so no roster consumes the rows; our
        // own channel still reaches every member through our beat.
    }
}

#[cfg(test)]
#[path = "own_voice_tests.rs"]
mod tests;
