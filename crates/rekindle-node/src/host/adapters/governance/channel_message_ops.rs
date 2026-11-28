//! A moderator's deletes of channel messages, our typing notice and the
//! channel we are in, over [`HostGovernance`] (plan E3.1 E2d), on both
//! modes.
//!
//! A delete is the durable `GovernanceEntry::AdminDelete`, which every
//! reader validates (`MANAGE_MESSAGES` and, plan E3.3 M1, a role no lower
//! than the post's author's, `validate_write`); the author is the one our
//! stored row names, and the delete carries it. It is checked here with
//! that same function before it is written, so a refusal answers
//! at once instead of being dropped by every peer. Our row becomes the
//! tombstone every reader stores from the merged op (plan E3.3 M4), and
//! the window hears `ChannelMessageEvent::Deleted` naming us.
//!
//! The channel we are in is the store's `my_session_location`, which our
//! presence row carries; it is republished at once by one presence poll
//! (the shell's own until presence moves at E3.4, `rekindled`'s presence
//! adapter otherwise).

use rekindle_governance::state::GovernanceState;
use rekindle_governance::validate::validate_write;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::presence::SessionLocation;

use super::moderation::{pseudonym, ModerationRefusal};
use super::HostGovernance;

/// A message id's 16 bytes: `msg_` and 32 hex digits, or a UUID in either
/// of its forms.
#[must_use]
pub fn message_id_bytes(message_id: &str) -> Option<[u8; 16]> {
    let bare = message_id.strip_prefix("msg_").unwrap_or(message_id);
    uuid::Uuid::try_parse(bare).ok().map(|id| *id.as_bytes())
}

/// Whether `me` may delete `author`'s messages in `channel`: the reader
/// rule over a probe `AdminDelete`.
///
/// # Errors
/// Readers would drop the entry.
pub fn may_delete_messages(
    state: &GovernanceState,
    me: &PseudonymKey,
    channel: ChannelId,
    author: &PseudonymKey,
) -> Result<(), ModerationRefusal> {
    let probe = GovernanceEntry::AdminDelete {
        message_id: [0; 16],
        channel_id: channel,
        author: author.clone(),
        reason: None,
    };
    if validate_write(me, &probe, state) {
        Ok(())
    } else {
        Err(ModerationRefusal::NotPermitted("delete members' messages"))
    }
}

/// One delete to write: the message's id as given, its bytes and its
/// author's pseudonym.
struct Target<'a> {
    id: &'a str,
    bytes: [u8; 16],
    author: PseudonymKey,
}

impl HostGovernance {
    /// The channel and the messages, once readers would accept our delete
    /// of each: a message we do not hold has no author we could name.
    async fn check_delete<'a>(
        &self,
        community_id: &str,
        channel_id: &str,
        message_ids: &'a [String],
    ) -> Result<(ChannelId, Vec<Target<'a>>), ModerationRefusal> {
        let channel = ChannelId::from_hex(channel_id).ok_or(ModerationRefusal::BadMessage)?;
        let ids = message_ids
            .iter()
            .map(|id| message_id_bytes(id).map(|bytes| (id, bytes)))
            .collect::<Option<Vec<_>>>()
            .ok_or(ModerationRefusal::BadMessage)?;
        let state = self
            .governance_state_impl(community_id)
            .ok_or(ModerationRefusal::NotAMember)?;
        let me = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .and_then(|hex| pseudonym(&hex))
            .ok_or(ModerationRefusal::NotAMember)?;
        let authors = self.post_authors(channel_id, message_ids).await?;
        let mut targets = Vec::with_capacity(ids.len());
        for ((id, bytes), author) in ids.into_iter().zip(authors) {
            let author = author.ok_or(ModerationRefusal::BadMessage)?;
            may_delete_messages(&state, &me, channel, &author)?;
            targets.push(Target { id, bytes, author });
        }
        Ok((channel, targets))
    }

    /// The author of each of `message_ids` in `channel_id`, as our stored
    /// row names it; `None` for one we do not hold.
    async fn post_authors(
        &self,
        channel_id: &str,
        message_ids: &[String],
    ) -> Result<Vec<Option<PseudonymKey>>, ModerationRefusal> {
        let (owner, db) = self.owner_and_db().ok_or(ModerationRefusal::NotAMember)?;
        let channel = channel_id.to_owned();
        let ids = message_ids.to_vec();
        let senders = db
            .call(move |conn| {
                let mut senders = Vec::with_capacity(ids.len());
                for id in &ids {
                    let row = rekindle_db::repo::messages::channel::find_by_message_id(
                        conn, &owner, &channel, id,
                    )?;
                    senders.push(row.map(|row| row.sender_key));
                }
                Ok(senders)
            })
            .await
            .map_err(|e| {
                ModerationRefusal::Failed(GovernanceRuntimeError::Adapter(format!(
                    "read message authors: {e}"
                )))
            })?;
        Ok(senders
            .into_iter()
            .map(|sender| sender.as_deref().and_then(pseudonym))
            .collect())
    }

    /// Delete member message `message_id` in `channel_id`.
    ///
    /// # Errors
    /// Readers would refuse it, an id is malformed, or the write failed.
    pub async fn admin_delete_message(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        reason: Option<&str>,
    ) -> Result<(), ModerationRefusal> {
        let ids = [message_id.to_owned()];
        let (channel, targets) = (self.check_delete(community_id, channel_id, &ids)).await?;
        for target in &targets {
            self.write_delete(community_id, channel, channel_id, target, reason)
                .await?;
        }
        Ok(())
    }

    /// Delete each of `message_ids` in `channel_id`: how many were written.
    /// One that fails is logged and the rest still go; peers see what we
    /// wrote.
    ///
    /// # Errors
    /// Readers would refuse it, or an id is malformed.
    pub async fn bulk_delete_messages(
        &self,
        community_id: &str,
        channel_id: &str,
        message_ids: &[String],
        reason: Option<&str>,
    ) -> Result<u32, ModerationRefusal> {
        let (channel, targets) = (self.check_delete(community_id, channel_id, message_ids)).await?;
        let mut deleted = 0u32;
        for target in &targets {
            match self
                .write_delete(community_id, channel, channel_id, target, reason)
                .await
            {
                Ok(()) => deleted += 1,
                Err(e) => tracing::warn!(
                    community = %community_id,
                    channel = %channel_id,
                    message_id = %target.id,
                    error = %e,
                    "bulk delete: one message was not deleted",
                ),
            }
        }
        Ok(deleted)
    }

    /// The `AdminDelete` entry, then our row's tombstone and the window's
    /// event, for one message.
    async fn write_delete(
        &self,
        community_id: &str,
        channel: ChannelId,
        channel_id: &str,
        target: &Target<'_>,
        reason: Option<&str>,
    ) -> Result<(), ModerationRefusal> {
        rekindle_governance_runtime::apply::write_entry(
            self,
            community_id,
            GovernanceEntry::AdminDelete {
                message_id: target.bytes,
                channel_id: channel,
                author: target.author.clone(),
                reason: reason.map(str::to_owned),
            },
        )
        .await?;
        // The entry is durable and every reader applies it from its merge;
        // our row is our copy, stored the same way.
        let me = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .and_then(|hex| pseudonym(&hex))
            .ok_or(ModerationRefusal::NotAMember)?;
        let author = hex::encode(target.author.0);
        let stored = self.store_admin_delete(community_id, channel_id, target.id, &me, &author);
        if let Err(e) = stored.await {
            tracing::warn!(community = %community_id, error = %e, "admin delete: stored row kept");
        }
        Ok(())
    }

    /// We are in `channel_id` (a voice channel when `voice`), or in none:
    /// our presence carries it from the next write, made now.
    ///
    /// # Errors
    /// The community is not joined.
    pub fn set_active_channel(
        &self,
        community_id: &str,
        channel_id: Option<String>,
        voice: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        let location = channel_id.map(|channel_id| {
            if voice {
                SessionLocation::Voice { channel_id }
            } else {
                SessionLocation::Text { channel_id }
            }
        });
        {
            let mut communities = self.ctx.communities.map().write();
            let community = communities.get_mut(community_id).ok_or_else(|| {
                GovernanceRuntimeError::CommunityNotFound(community_id.to_owned())
            })?;
            community.my_session_location = location;
        }
        self.spawn_presence_poll_tick_impl(community_id);
        Ok(())
    }
}

#[cfg(test)]
#[path = "channel_message_ops_tests.rs"]
mod tests;
