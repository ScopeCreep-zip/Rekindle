//! The sync loop's community phase (plan E3.1 C2b), one task per unlock on
//! both modes, moved from the desktop's `sync_service`. Every tick (three
//! at [`FIRST_TICK`], so peers found on the mesh soon after login hear us,
//! then every [`TICK`]) it re-announces our presence and route to each
//! community's gossip mesh. Its first tick with a route also asks for every
//! community key we hold behind ([`HostGovernance::recover_behind_meks`]);
//! every later tick asks again for a key still behind, at the next cascade
//! index ([`HostGovernance::repair_behind_meks`], plan E3.2 S7m item 1c).
//!
//! It reads no record (D4): a channel or governance record's change arrives
//! by its watch, so the desktop's per-tick inspect of every channel record
//! is gone, not moved. The ticks are tokio time (paused under netsim).
//! Channel history is each author's outbox, read from our cursors by the
//! outbox catch-up (plan E3.3); no peer is asked for it.

use std::sync::Arc;
use std::time::Duration;

use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_types::presence::SessionStatus;
use tokio_util::sync::CancellationToken;

use super::HostGovernance;
use crate::daemon::dispatch::DaemonContext;

/// The first ticks' period.
const FIRST_TICK: Duration = Duration::from_secs(10);
/// How many ticks run at [`FIRST_TICK`].
const FIRST_TICKS: u32 = 3;
/// The steady period.
const TICK: Duration = Duration::from_secs(30);

/// Start the community sync, the anti-entropy loop, the member routes
/// writer and the invite expiry timer in `scope` (the unlock's).
///
/// # Errors
/// The scope is closed (the unlock is ending).
pub(crate) fn start(
    ctx: &DaemonContext,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    let host = HostGovernance::new(ctx.this.upgrade().expect("a context in use is alive"));
    let expiry = host.clone();
    let reminders = host.clone();
    let markers = host.clone();
    let sweeps = host.clone();
    let routes = host.clone();
    scope.spawn_with_token("read marker write-behind", move |stop| {
        super::unread::write_behind(markers, stop)
    })?;
    scope.spawn_with_token("community sync", move |stop| run(host, stop))?;
    scope.spawn_with_token("governance anti-entropy", move |stop| {
        super::anti_entropy_loop::run(sweeps, stop)
    })?;
    scope.spawn_with_token("member routes", move |stop| {
        super::member_routes_loop::run(routes, stop)
    })?;
    scope.spawn_with_token("community invite expiry", move |stop| {
        super::invite_expiry::run(expiry, stop)
    })?;
    scope.spawn_with_token("community event reminders", move |stop| {
        super::event_reminders::run(reminders, stop)
    })
}

async fn run(host: HostGovernance, stop: CancellationToken) {
    let mut ticks = tokio::time::interval(FIRST_TICK);
    let mut count: u32 = 0;
    let mut recovered = false;
    // The cascade indices a repeated key request walks.
    let cascades = u32::try_from(rekindle_mek_rotation::MAX_CASCADES).unwrap_or(u32::MAX);
    while stop.run_until_cancelled(ticks.tick()).await.is_some() {
        count += 1;
        if count == FIRST_TICKS {
            ticks = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        }
        let route = host.our_route_blob_impl();
        // While our status hides presence the whole phase is silent: a
        // member that quit sends no presence and no key
        // request, and an Invisible one must not be told apart from it.
        // The first tick after un-hiding runs the normal catch-up.
        if route.is_empty() || rekindle_presence::hides_presence(host.ctx.own_presence.status()) {
            continue;
        }
        if recovered {
            host.repair_behind_meks(count % cascades);
        } else {
            host.recover_behind_meks();
            recovered = true;
        }
        for community_id in host.sync_targets() {
            if stop.is_cancelled() {
                return;
            }
            host.sync_community(&community_id, &route);
        }
    }
}

/// The wire form of our status in a community `PresenceUpdate` or
/// `MemberJoined`.
/// The re-announce of our presence and route, for a shown status.
fn presence_announce(
    status: SessionStatus,
    pseudonym_key: String,
    route: &[u8],
) -> CommunityEnvelope {
    CommunityEnvelope::PresenceUpdate {
        pseudonym_key,
        status: status_str(status).to_string(),
        game_info: None,
        route_blob: Some(route.to_vec()),
    }
}

pub(super) fn status_str(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Online => "online",
        SessionStatus::Away => "away",
        SessionStatus::Busy => "busy",
        SessionStatus::Offline | SessionStatus::Invisible => "offline",
    }
}

impl HostGovernance {
    /// The communities we hold governance for, with a pseudonym in each and
    /// not removed from (a removed member sends nothing, plan E3.2 S9), in
    /// id order (the store's map has no order; the phase's sends must not
    /// depend on it).
    pub(super) fn sync_targets(&self) -> Vec<String> {
        let mut targets: Vec<String> = self
            .ctx
            .communities
            .map()
            .read()
            .values()
            .filter(|c| c.governance_state.is_some() && c.my_pseudonym_key.is_some())
            .filter(|c| c.own_removal.is_none())
            .map(|c| c.id.clone())
            .collect();
        targets.sort_unstable();
        targets
    }

    /// One community's phase: our presence and route. Nothing at all while
    /// our status hides presence ([`rekindle_presence::hides_presence`]).
    pub(crate) fn sync_community(&self, community_id: &str, route: &[u8]) {
        let status = self.ctx.own_presence.status();
        if rekindle_presence::hides_presence(status) {
            return;
        }
        let Some(pseudonym) = (self.ctx.communities.map().read())
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
        else {
            return;
        };
        let presence = presence_announce(status, pseudonym, route);
        if let Err(e) = self.send_to_mesh(community_id, &presence) {
            tracing::debug!(community = %community_id, error = %e, "presence re-announce not sent");
        }
    }
}
