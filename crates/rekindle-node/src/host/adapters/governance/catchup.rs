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

    /// A merge created a channel (plan E3.3, `channel::outbox_unmapped`):
    /// diff again, on the community's scope, every outbox whose entries
    /// mapped to no merged channel since the last creation.
    pub(super) fn spawn_outbox_reread_unmapped(&self, community_id: &str) {
        let channel = crate::host::adapters::channel::HostChannel::new(&self.ctx);
        let community = community_id.to_string();
        self.ctx.community_scope(community_id).spawn_or_drop(
            "channel outbox re-read for a new channel",
            async move {
                channel.reread_unmapped(&community).await;
            },
        );
    }

    /// An older community key arrived (plan D16, `key_history.rs`): forget
    /// the community's outbox cursors, so the entries a catch-up passed for
    /// want of that key are read again, then catch every outbox up.
    pub(super) fn spawn_outbox_reread(&self, community_id: &str) {
        let channel = crate::host::adapters::channel::HostChannel::new(&self.ctx);
        let owner_and_db = self.owner_and_db();
        let community = community_id.to_string();
        self.ctx.community_scope(community_id).spawn_or_drop(
            "channel outbox re-read",
            async move {
                let Some((owner, db)) = owner_and_db else {
                    return;
                };
                let stream = crate::host::adapters::channel::outbox_stream(&community);
                if let Err(error) = db
                    .call(move |conn| {
                        rekindle_db::repo::stream_cursors::reset_stream(conn, &owner, &stream)
                    })
                    .await
                {
                    tracing::warn!(community_id = %community, %error, "outbox cursors not reset");
                    return;
                }
                let (authors, applied) = channel.catch_up_outboxes(&community).await;
                tracing::info!(community_id = %community, authors, applied, "channel outbox re-read ended (D16)");
            },
        );
    }
}
