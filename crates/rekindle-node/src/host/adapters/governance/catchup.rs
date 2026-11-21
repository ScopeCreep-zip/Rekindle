//! A community's history catch-up, on both hosts: every member's channel
//! outbox read from our `stream_cursors` cursors (plan E3.3 "Cursor and
//! authors"), on load, join and every key install. It runs on the
//! community's scope, so leaving or locking stops it. No peer is asked for
//! history: every post is an entry of its author's never-retired `M`/4
//! chain (plan E3.3 slice 8 deleted the `SyncRequest` ask).

use super::HostGovernance;

impl HostGovernance {
    /// Catch up every member's channel outbox in `community_id` from our
    /// cursors, on the community's scope (plan E3.3 "Cursor and authors").
    pub(super) fn spawn_outbox_catchup(&self, community_id: &str) {
        let channel = crate::host::adapters::channel::HostChannel::new(&self.ctx);
        let community = community_id.to_string();
        self.ctx.community_scope(community_id).spawn_or_drop(
            "channel outbox catch-up",
            async move {
                tracing::info!(community_id = %community, "channel outbox catch-up started");
                let (authors, applied) = channel.catch_up_outboxes(&community).await;
                tracing::info!(community_id = %community, authors, applied, "channel outbox catch-up ended");
            },
        );
    }
}
