//! The channel a message change names, and a moderator's deletes on our
//! rows (plan E3.3 M4). An author's own edits and deletes are entries in
//! its record, the channel adapter's `change_own_message`; a moderator's
//! delete is the governance op `AdminDelete`, which every reader applies
//! from the merged state as a tombstone naming the moderator, under
//! delete-wins: a post that lands after it is stored removed.

use rekindle_db::repo::messages::channel_changes::{self as change_rows, AdminTombstone};
use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

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

/// The moderator whose merged `AdminDelete` removed
/// `message_id` of `channel_id` by `author` (pseudonym hex), when one did
/// and still may (M2: a delete the current state no longer grants
/// soft-fails). The delete names its channel and the post's author, and
/// only a channel of this community's state and the post's own author
/// count, so a moderator elsewhere cannot remove a post here by its id,
/// nor a post of a higher-ranked author by naming a lower one (M1).
pub(crate) fn removed_by(
    state: &GovernanceState,
    channel_id: &str,
    message_id: &str,
    author: &str,
) -> Option<PseudonymKey> {
    let bytes = super::channel_message_ops::message_id_bytes(message_id)?;
    let delete = state.admin_deletes.get(&bytes)?;
    let granted = delete.channel_id.to_hex() == channel_id
        && state.channels.contains_key(&delete.channel_id)
        && super::moderation::pseudonym(author).as_ref() == Some(&delete.author)
        && super::channel_message_ops::may_delete_messages(
            state,
            &delete.by,
            delete.channel_id,
            &delete.author,
        )
        .is_ok();
    granted.then(|| delete.by.clone())
}

/// One removal to store: the channel, the message, the moderator and the
/// post's author the delete names.
struct Removal {
    channel_id: String,
    message_id: String,
    by: String,
    author: String,
}

impl HostGovernance {
    /// Apply every moderator delete of `community_id`'s merged state to
    /// our rows (each merge may bring new ones), and tell the window about
    /// each row that changed. A row already deleted does not change.
    pub(crate) async fn apply_admin_deletes(&self, community_id: &str) {
        let Some(state) = self.governance_state_impl(community_id) else {
            return;
        };
        let removals: Vec<Removal> = (state.admin_deletes.iter())
            .filter_map(|(bytes, delete)| {
                let channel_id = delete.channel_id.to_hex();
                let message_id = format!("msg_{}", hex::encode(bytes));
                let author = hex::encode(delete.author.0);
                let by = hex::encode(removed_by(&state, &channel_id, &message_id, &author)?.0);
                Some(Removal {
                    channel_id,
                    message_id,
                    by,
                    author,
                })
            })
            .collect();
        if removals.is_empty() {
            return;
        }
        let now = self.ctx.clock.now_secs();
        match self.store_removals(removals, now).await {
            Ok(changed) => {
                for removal in changed {
                    self.publish_removed(community_id, removal);
                }
            }
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "moderator deletes not stored");
            }
        }
    }

    /// Store a moderator's delete by `by` of `message_id`, a post by
    /// `author` (pseudonym hex), on our row and tell the window, when the
    /// row changed.
    ///
    /// # Errors
    /// No identity is unlocked, or the store failed.
    pub(crate) async fn store_admin_delete(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        by: &PseudonymKey,
        author: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        let removal = Removal {
            channel_id: channel_id.to_owned(),
            message_id: message_id.to_owned(),
            by: hex::encode(by.0),
            author: author.to_owned(),
        };
        let now = self.ctx.clock.now_secs();
        for removal in self.store_removals(vec![removal], now).await? {
            self.publish_removed(community_id, removal);
        }
        Ok(())
    }

    /// Tombstone each removal's row: the ones that changed.
    async fn store_removals(
        &self,
        removals: Vec<Removal>,
        now: u64,
    ) -> Result<Vec<Removal>, GovernanceRuntimeError> {
        let (owner, db) = self
            .owner_and_db()
            .ok_or_else(|| adapter("no identity unlocked"))?;
        db.call(move |conn| {
            let mut changed = Vec::new();
            for removal in removals {
                let delete = AdminTombstone {
                    owner_key: &owner,
                    channel_id: &removal.channel_id,
                    message_id: &removal.message_id,
                    by: &removal.by,
                    author: &removal.author,
                    deleted_at: now,
                };
                if change_rows::admin_tombstone(conn, &delete)? {
                    changed.push(removal);
                }
            }
            Ok(changed)
        })
        .await
        .map_err(|e| adapter(format!("store moderator delete: {e}")))
    }

    fn publish_removed(&self, community_id: &str, removal: Removal) {
        self.ctx.publish_event(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::Deleted {
                community: community_id.to_owned(),
                channel: removal.channel_id,
                message_id: removal.message_id,
                removed_by: Some(removal.by),
            },
        ));
    }
}
