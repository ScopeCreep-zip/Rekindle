//! The daemon's voice state for its registry poll (plan E3.4 S8-pre),
//! until S8b deletes the poll: no voice session. The Host's beat path reads
//! no port on `rekindled` (its beats carry no voice channel).

use rekindle_presence::deps::VoicePresenceRow;

pub(super) struct DaemonSessionPresence;

impl rekindle_presence::SessionPresence for DaemonSessionPresence {
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
