mod poll;
pub(crate) mod registry;
mod sync;

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Wire-format presence string for the local user, scoped to a
/// specific community. Returns "online"/"away"/"busy"/"offline"
/// based on the global identity status today; the `community_id`
/// parameter is the wire-up point for per-community
/// `MemberPresence.custom_status` (architecture
/// `.claude/docs/rekindle-communities-architecture.md` line 754).
///
/// Kept as a public src-tauri helper so commands that need to read
/// the local user's effective community-presence string (status
/// pickers, settings panels, the planned `getMyCommunityPresence`
/// IPC) can call it without going through the deps trait + adapter
/// dance. Internal presence orchestrators consume the same value via
/// `CommunityPresenceDeps::current_presence_status_str`.
pub fn current_presence_status(state: &Arc<AppState>, _community_id: &str) -> &'static str {
    match state_helpers::identity_status(state).unwrap_or(crate::state::UserStatus::Online) {
        crate::state::UserStatus::Online => "online",
        crate::state::UserStatus::Away => "away",
        crate::state::UserStatus::Busy => "busy",
        crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => "offline",
    }
}

pub use poll::{presence_poll_tick_public, start_presence_poll};
pub use registry::write_our_presence;
pub use sync::run_initial_sync;
