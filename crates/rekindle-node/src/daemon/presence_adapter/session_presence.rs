//! The daemon's session signals for its registry poll (plan E3.4 S8-pre),
//! until S8b deletes the poll: our status, nothing shared, no voice
//! session. The Host's beat path reads no port on `rekindled` (its beats
//! carry the status alone).

use std::sync::Arc;

use rekindle_presence::deps::VoicePresenceRow;
use rekindle_types::presence::{MemberSession, PresenceSharingPolicy};

use crate::daemon::dispatch::DaemonContext;

pub(super) struct DaemonSessionPresence {
    pub(super) ctx: Arc<DaemonContext>,
}

impl rekindle_presence::SessionPresence for DaemonSessionPresence {
    fn self_session(&self, _community_id: &str) -> MemberSession {
        // Our status, from the host's own presence (plan E1.1); location
        // and activity stay unshared (the default policy below).
        MemberSession {
            status: self.ctx.own_presence.status(),
            ..MemberSession::default()
        }
    }

    fn presence_policy(&self, _community_id: &str) -> PresenceSharingPolicy {
        // The daemon shares liveness and nothing else. Location and
        // activity are user-consented signals and there is no user at a
        // keyboard here to consent.
        PresenceSharingPolicy::default()
    }

    fn active_voice_channel(&self, _community_id: &str) -> Option<String> {
        // The daemon *does* have voice IPC (`VoiceJoin` / `VoiceLeave`),
        // so this is not "no voice engine" — it is that nothing records
        // the session. `dispatch::presence::handle_voice_join` calls
        // `operations::voice::join_voice`, returns the session in its
        // IPC reply, and stores none of it; `handle_voice_leave` takes
        // `_ctx` and tears nothing down. Until that state exists there
        // is no honest answer here, and inventing one would put a
        // member in a voice channel the daemon cannot leave.
        None
    }

    fn reconcile_voice_roster(&self, _community_id: &str, _rows: Vec<VoicePresenceRow>) {}
}
