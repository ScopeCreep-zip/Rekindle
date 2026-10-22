//! The sync loop's community phase (plan E3.1 C2b), one task per unlock on
//! both modes, moved from the desktop's `sync_service`. Every tick (three
//! at [`FIRST_TICK`], so peers found on the mesh soon after login hear us,
//! then every [`TICK`]) it re-announces our presence and route to each
//! community's gossip mesh and asks the mesh for each channel's messages
//! since our newest. Its first tick with a route also asks for every
//! community key we lack or hold behind ([`HostGovernance::recover_behind_meks`]).
//!
//! It reads no record (D4): a channel or governance record's change arrives
//! by its watch, so the desktop's per-tick inspect of every channel record
//! is gone, not moved. The ticks are tokio time (paused under netsim).
//! History catch-up from the channel records themselves is the login
//! network stage's (C2a), not this loop's.

use std::sync::Arc;
use std::time::Duration;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
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

/// Start the community sync and the invite expiry timer in `scope` (the
/// unlock's).
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
    scope.spawn_with_token("read marker write-behind", move |stop| {
        super::unread::write_behind(markers, stop)
    })?;
    scope.spawn_with_token("community sync", move |stop| run(host, stop))?;
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
    while stop.run_until_cancelled(ticks.tick()).await.is_some() {
        count += 1;
        if count == FIRST_TICKS {
            ticks = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        }
        let route = host.our_route_blob_impl();
        if route.is_empty() {
            continue;
        }
        if !recovered {
            host.recover_behind_meks();
            recovered = true;
        }
        for community_id in host.sync_targets() {
            if stop.is_cancelled() {
                return;
            }
            host.sync_community(&community_id, &route).await;
        }
    }
}

/// The wire form of our status in a community `PresenceUpdate` or
/// `MemberJoined`.
pub(super) fn status_str(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Online => "online",
        SessionStatus::Away => "away",
        SessionStatus::Busy => "busy",
        SessionStatus::Offline | SessionStatus::Invisible => "offline",
    }
}

impl HostGovernance {
    /// The communities we hold governance for, with a pseudonym in each, in
    /// id order (the store's map has no order; the phase's sends must not
    /// depend on it).
    fn sync_targets(&self) -> Vec<String> {
        let mut targets: Vec<String> = self
            .ctx
            .communities
            .map()
            .read()
            .values()
            .filter(|c| c.governance_state.is_some() && c.my_pseudonym_key.is_some())
            .map(|c| c.id.clone())
            .collect();
        targets.sort_unstable();
        targets
    }

    /// One community's phase: our presence and route, then a `SyncRequest`
    /// per channel since our newest message in it.
    pub(crate) async fn sync_community(&self, community_id: &str, route: &[u8]) {
        let (pseudonym, channels) = {
            let map = self.ctx.communities.map().read();
            let Some(c) = map.get(community_id) else {
                return;
            };
            let Some(pseudonym) = c.my_pseudonym_key.clone() else {
                return;
            };
            let mut channels: Vec<String> = c.channel_log_keys.keys().cloned().collect();
            channels.sort_unstable();
            (pseudonym, channels)
        };
        let presence = CommunityEnvelope::PresenceUpdate {
            pseudonym_key: pseudonym,
            status: status_str(self.ctx.own_presence.status()).to_string(),
            game_info: None,
            route_blob: Some(route.to_vec()),
        };
        if let Err(e) = self.send_to_mesh(community_id, &presence) {
            tracing::debug!(community = %community_id, error = %e, "presence re-announce not sent");
            return;
        }
        for channel_id in channels {
            let Some(since) = self.last_channel_timestamp(community_id, &channel_id).await else {
                continue;
            };
            let request = CommunityEnvelope::Control(ControlPayload::SyncRequest {
                channel_id: channel_id.clone(),
                since_timestamp: since,
            });
            if self.send_to_mesh(community_id, &request).is_err() {
                return;
            }
            let now = self.ctx.clock.now_secs();
            if let Some(c) = self.ctx.communities.map().write().get_mut(community_id) {
                c.pending_syncs.insert(channel_id, (now, 1));
            }
        }
    }

    /// Our newest message's timestamp in the channel (0 for none); `None`
    /// when it cannot be read, so no request claims a wrong watermark.
    async fn last_channel_timestamp(&self, community_id: &str, channel_id: &str) -> Option<u64> {
        let owner = self.ctx.require_owner().ok()?;
        let db = self.ctx.db.current().ok()?;
        let channel = channel_id.to_string();
        match db
            .call(move |c| rekindle_db::repo::messages::last_channel_timestamp(c, &owner, &channel))
            .await
        {
            Ok(ts) => Some(ts.cast_unsigned()),
            Err(e) => {
                tracing::warn!(community = %community_id, channel = %channel_id, error = %e, "channel watermark unreadable");
                None
            }
        }
    }
}
