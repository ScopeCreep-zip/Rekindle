//! The registry arm of the Host's watch path (plan E3.4 S5, D4), on both
//! modes: a ValueChange on a community's member registry (or a Plate Gate
//! segment's) is a presence change, answered by a presence *refresh*: read
//! the rows and merge them, never write ours. Only the poll's 60 s beat
//! writes our row (`rekindle_presence::community::spawn`).
//!
//! The policy lived in the desktop (`sync_communities.rs`) until here, so
//! the netsim could not reach it (IBS "Owed"). A write in this reaction was
//! the round-7 Linux storm (finding IB1c): each member's row write fired
//! the other's registry watch, whose tick wrote again, ~45 writes a minute
//! on both sides. Netsim `presence_idle_writes` bounds the idle writes.
//!
//! The Host decides; the run is the presence poll's owner: the shell's own
//! presence adapter until the poll moves into the Host (S8b), else the
//! daemon's (`DaemonPresenceAdapter`), as
//! [`spawn_presence_poll_tick_impl`](super::HostGovernance) does.

use std::sync::Arc;

use super::HostGovernance;

impl HostGovernance {
    /// The community whose member registry, or a Plate Gate segment
    /// registry, is `record`; the lowest id when several name it.
    fn community_of_registry(&self, record: &str) -> Option<String> {
        let map = self.ctx.communities.map().read();
        let mut ids: Vec<&String> = map
            .values()
            .filter(|c| {
                c.member_registry_key.as_deref() == Some(record)
                    || c.segments.iter().any(|s| s.registry_key == record)
            })
            .map(|c| &c.id)
            .collect();
        ids.sort_unstable();
        ids.first().map(|id| (*id).clone())
    }

    /// Handle a watched record's change when it is a community's registry:
    /// `true` when it was one, `false` for any other record. A community we
    /// were removed from is read-only history: its registry change reads
    /// nothing (plan E3.2 S9, R3-1).
    pub(super) async fn on_registry_change(&self, record: &str) -> bool {
        let Some(community) = self.community_of_registry(record) else {
            return false;
        };
        if self.ctx.communities.own_removal(&community).is_some() {
            return true;
        }
        tracing::debug!(%community, record, "registry changed — presence refresh, no write");
        self.presence_refresh(&community).await;
        true
    }

    /// Read and merge `community_id`'s registry rows without writing ours:
    /// the shell's presence adapter while the shell runs the polls, else
    /// the daemon's, in the community's scope (a leave stops it).
    async fn presence_refresh(&self, community_id: &str) {
        if let Some(shell) = &self.ctx.shell {
            shell.presence_refresh(community_id).await;
            return;
        }
        let adapter = Arc::new(crate::daemon::presence_adapter::DaemonPresenceAdapter::new(
            Arc::clone(&self.ctx),
        ));
        let community = community_id.to_string();
        self.ctx
            .community_scope(community_id)
            .spawn_or_drop("presence refresh", async move {
                if let Err(e) = rekindle_presence::presence_refresh(adapter, &community).await {
                    tracing::debug!(community = %community, error = %e, "presence refresh after a registry change failed");
                }
            });
    }
}
