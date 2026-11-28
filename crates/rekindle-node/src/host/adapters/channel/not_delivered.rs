//! Our posts that did not reach their readers, failed with the reason the
//! user reads (plan E3.2 S9, finding F9-3; S7l's key deadline, F9-1): the
//! stored row reads `failed` with it, so a reload shows why, and every
//! window is told with the same `ChannelSendStatus` a failed write sends.

use rekindle_db::repo::messages::channel as rows;
use rekindle_types::message::SendState;

use super::HostChannel;
use crate::host::adapters::governance::HostGovernance;

/// One of our posts: its channel and its message id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnPost {
    pub channel: String,
    pub message: String,
}

impl HostChannel {
    /// Our post `message` reads `failed` with `reason` in its stored row;
    /// whether a row changed.
    pub(super) async fn store_failure_reason(&self, message: &str, reason: &str) -> bool {
        let Ok((owner, db)) = self.owner_and_db() else {
            return false;
        };
        let (message, reason) = (message.to_owned(), reason.to_owned());
        db.call(move |conn| rows::fail_own_post(conn, &owner, &message, &reason))
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "channel post failure not stored"))
            .unwrap_or(false)
    }
}

/// Fail `posts` of `community_id` with `reason`: each stored row of ours
/// reads `failed` with it, and the windows are told. A post that is not
/// ours, or already delivered elsewhere, is left alone. Returns how many
/// failed.
pub(crate) async fn fail_not_delivered(
    gov: &HostGovernance,
    community_id: &str,
    posts: &[OwnPost],
    reason: &str,
) -> usize {
    let channel = HostChannel { gov: gov.clone() };
    let mut failed = 0;
    for post in posts {
        if channel.store_failure_reason(&post.message, reason).await {
            failed += 1;
            channel.send_state_impl(
                community_id,
                &post.channel,
                &post.message,
                SendState::Failed,
                Some(reason.to_owned()),
            );
        }
    }
    failed
}
