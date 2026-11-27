//! Our own session in a community, as the Host holds it (plan E3.4 S8-pre,
//! ruled by the lead 2026-11-26 on `58eb04bd7`: every frontend is a thin
//! wrapper over the shared backend). Our status and game are our own
//! presence's (`host::own_presence`, plan E1.1d), the channel we are in is
//! the community's (`my_session_location`), and our sharing consent is the
//! community's `presence_policy` (`set_presence_policy`, S5d). Our beat seals
//! its extras from them on both modes, a beat's extras are opened against
//! that consent, and the shell's registry row reads the same until S8a; no
//! shell holds a copy.

use rekindle_types::presence::{MemberSession, PresenceSharingPolicy};

use crate::daemon::dispatch::DaemonContext;

/// Our session in `community`: our status and our game (the activity our
/// consent may let out), the channel we are in, and now as last-active.
pub(crate) fn own_session(ctx: &DaemonContext, community: &str) -> MemberSession {
    let own = ctx.own_presence.view();
    let location = (ctx.communities.map().read())
        .get(community)
        .and_then(|c| c.my_session_location.clone());
    MemberSession {
        status: own.status,
        location,
        activity: own.game.map(|game| game.game_name),
        last_active: ctx.clock.now_secs(),
    }
}

/// Our sharing consent in `community`; default-deny in a community we do
/// not hold.
pub(crate) fn presence_policy(ctx: &DaemonContext, community: &str) -> PresenceSharingPolicy {
    stored_policy(ctx, community).unwrap_or_default()
}

/// The policy held for a joined community.
pub(crate) fn stored_policy(ctx: &DaemonContext, community: &str) -> Option<PresenceSharingPolicy> {
    ctx.communities
        .map()
        .read()
        .get(community)
        .map(|c| c.presence_policy.clone())
}
