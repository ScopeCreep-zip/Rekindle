//! The D25 unread engine (plan E3.1 G1), on both hosts.
//!
//! A channel's read marker is the lamport of the last message read; it only
//! advances, in memory at once (`DaemonContext::read_markers`) and in
//! `read_markers` within 30 s ([`write_behind`]). Counts are derived from the
//! database — the received messages after the marker, and of those the ones
//! that mention us — and every change is published as `UnreadChanged`.
//! Markers stay private: no member is ever sent one.

use std::time::Duration;

use rekindle_db::repo::read_markers::{self, Scope, Unread};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::dto::community::ChannelUnreadView;
use rekindle_types::subscription_events::{SubscriptionEvent, UnreadContext};
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

/// The longest a marker waits in memory before it is written.
pub(crate) const WRITE_BEHIND: Duration = Duration::from_secs(30);

/// Write pending markers every [`WRITE_BEHIND`], and once more as the unlock
/// ends.
pub(super) async fn write_behind(host: HostGovernance, stop: CancellationToken) {
    let mut ticks = tokio::time::interval(WRITE_BEHIND);
    while stop.run_until_cancelled(ticks.tick()).await.is_some() {
        host.flush_read_markers().await;
    }
    host.flush_read_markers().await;
}

fn channel_scope(community_id: &str, channel_id: &str) -> Scope {
    Scope::Channel {
        community_id: community_id.to_owned(),
        channel_id: channel_id.to_owned(),
    }
}

fn stored(e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("read markers: {e}"))
}

impl HostGovernance {
    /// Advance our marker on `channel_id` to the message `message_id`, and
    /// publish the channel's count. A position at or below the marker
    /// changes nothing.
    ///
    /// # Errors
    /// Locked, the message is not stored here, or the database failed.
    pub(crate) async fn mark_channel_read(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        let (owner, db) = self.owner_db()?;
        let (channel, message) = (channel_id.to_owned(), message_id.to_owned());
        let position = db
            .call(move |conn| read_markers::channel_position(conn, &owner, &channel, &message))
            .await
            .map_err(stored)?
            .ok_or_else(|| stored(format!("message {message_id} is not stored")))?;
        let scope = channel_scope(community_id, channel_id);
        if position > self.marker(&scope).await? {
            self.ctx.read_markers.note(scope, position);
        }
        self.publish_channel_unread(community_id, channel_id).await
    }

    /// Each channel of `community_id`, in channel order, with its unread
    /// messages and mentions.
    ///
    /// # Errors
    /// Locked, or the database failed.
    pub(crate) async fn unread_counts(
        &self,
        community_id: &str,
    ) -> Result<Vec<ChannelUnreadView>, GovernanceRuntimeError> {
        let channels: Vec<String> = self
            .ctx
            .communities
            .map()
            .read()
            .get(community_id)
            .map(|c| c.channels.iter().map(|ch| ch.id.clone()).collect())
            .unwrap_or_default();
        let mut out = Vec::with_capacity(channels.len());
        for channel_id in channels {
            let unread = self.channel_unread(community_id, &channel_id).await?;
            out.push(ChannelUnreadView {
                channel_id,
                unread_count: unread.messages,
                mention_count: unread.mentions,
            });
        }
        Ok(out)
    }

    /// A message was stored in `channel_id`: publish its count.
    ///
    /// # Errors
    /// Locked, or the database failed.
    pub async fn publish_channel_unread(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        let unread = self.channel_unread(community_id, channel_id).await?;
        self.ctx.publish_event(SubscriptionEvent::UnreadChanged {
            context: UnreadContext::Channel {
                community: community_id.to_owned(),
                channel: channel_id.to_owned(),
            },
            count: unread.messages,
        });
        Ok(())
    }

    /// The received messages of `channel_id` after our marker.
    async fn channel_unread(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<Unread, GovernanceRuntimeError> {
        let after = self
            .marker(&channel_scope(community_id, channel_id))
            .await?;
        let (owner, db) = self.owner_db()?;
        let (community, channel) = (community_id.to_owned(), channel_id.to_owned());
        db.call(move |conn| read_markers::channel_unread(conn, &owner, &community, &channel, after))
            .await
            .map_err(stored)
    }

    /// Our marker on `scope`: the larger of the stored and the pending one.
    async fn marker(&self, scope: &Scope) -> Result<u64, GovernanceRuntimeError> {
        let pending = self.ctx.read_markers.get(scope).unwrap_or(0);
        let (owner, db) = self.owner_db()?;
        let scope = scope.clone();
        let stored_seq = db
            .call(move |conn| read_markers::get(conn, &owner, &scope))
            .await
            .map_err(stored)?;
        Ok(stored_seq.max(pending))
    }

    /// Write every pending marker. Markers that could not be written wait
    /// for the next pass.
    pub(crate) async fn flush_read_markers(&self) {
        let pending = self.ctx.read_markers.take();
        if pending.is_empty() {
            return;
        }
        let Ok((owner, db)) = self.owner_db() else {
            return;
        };
        let now = self.ctx.clock.now_ms_i64();
        let batch = pending.clone();
        let written = db
            .call(move |conn| {
                let tx = conn.transaction()?;
                for (scope, seq) in &batch {
                    read_markers::advance(&tx, &owner, scope, *seq, now)?;
                }
                tx.commit()
            })
            .await;
        if let Err(e) = written {
            tracing::warn!(error = %e, "read markers not written; kept for the next pass");
            for (scope, seq) in pending {
                self.ctx.read_markers.note(scope, seq);
            }
        }
    }
}
