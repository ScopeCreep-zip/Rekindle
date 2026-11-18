//! The channel a message change names, and the moderator delete's row
//! removal. An author's own edits and deletes are entries in its record
//! (plan E3.3), the channel adapter's `change_own_message`.

use rekindle_governance_runtime::GovernanceRuntimeError;

use super::HostGovernance;
use crate::daemon::dispatch::DaemonContext;

/// The joined community holding `channel_id` (a hex channel id), read
/// from the store; channel ids are unique, so at most one matches.
pub(crate) fn community_of_channel(ctx: &DaemonContext, channel_id: &str) -> Option<String> {
    ctx.communities
        .map()
        .read()
        .iter()
        .find(|(_, community)| community.channels.iter().any(|ch| ch.id == channel_id))
        .map(|(id, _)| id.clone())
}

fn adapter(message: impl Into<String>) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(message.into())
}

impl HostGovernance {
    /// Hard-delete the stored row of `message_id` (the moderator delete).
    pub(super) async fn delete_stored(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        let (owner, db) = self
            .owner_and_db()
            .ok_or_else(|| adapter("no identity unlocked"))?;
        let (channel, message) = (channel_id.to_string(), message_id.to_string());
        db.call(move |conn| {
            rekindle_db::repo::messages::channel::delete(conn, &owner, &channel, &message)
        })
        .await
        .map(drop)
        .map_err(|e| adapter(format!("delete stored message: {e}")))
    }
}
