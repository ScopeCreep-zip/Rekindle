//! Edits and deletes of our own channel messages over [`HostGovernance`]
//! (plan E3.1 E2), on both modes: the change is gossiped to the
//! community's mesh as `ControlPayload::MessageEdited` / `MessageDeleted`
//! (the record entry stays; every reader applies the control), and a
//! delete also drops our stored row. Our own frontends get the same
//! `Edited` / `Deleted` event a receiver's do (plan E7.13 M45): the mesh
//! never echoes a frame to its sender, so without it the author's own
//! window kept the old body (Signal, Discord: the author's view changes
//! at once).

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::id::ChannelId;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::HostGovernance;
use crate::daemon::dispatch::DaemonContext;

/// What happens to the message.
#[derive(Debug, Clone)]
pub(crate) enum MessageChange {
    /// Its body becomes `body`.
    Edit { body: String },
    /// It is deleted.
    Delete,
}

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
    /// Apply `change` to our message `message_id` in `channel_id` of
    /// `community_id`.
    ///
    /// # Errors
    /// No current text key for the channel (edit), the seal failed, or
    /// the stored row could not be deleted.
    pub(crate) async fn change_channel_message(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        change: MessageChange,
    ) -> Result<(), GovernanceRuntimeError> {
        let (community, channel, message) = (
            community_id.to_string(),
            channel_id.to_string(),
            message_id.to_string(),
        );
        let (payload, own_event) = match change {
            MessageChange::Edit { body } => {
                let (new_ciphertext, mek_generation) =
                    self.seal_edit(community_id, channel_id, &body)?;
                let edited_at = self.ctx.clock.now_secs();
                let payload = ControlPayload::MessageEdited {
                    channel_id: channel.clone(),
                    message_id: message.clone(),
                    new_ciphertext,
                    mek_generation,
                    edited_at,
                };
                let event = ChannelMessageEvent::Edited {
                    community,
                    channel,
                    message_id: message,
                    body: Some(body),
                    edited_at,
                };
                (payload, event)
            }
            MessageChange::Delete => {
                self.delete_stored(channel_id, message_id).await?;
                let payload = ControlPayload::MessageDeleted {
                    channel_id: channel.clone(),
                    message_id: message.clone(),
                };
                let event = ChannelMessageEvent::Deleted {
                    community,
                    channel,
                    message_id: message,
                };
                (payload, event)
            }
        };
        self.send_to_mesh(community_id, &CommunityEnvelope::Control(payload))?;
        self.ctx
            .publish_event(SubscriptionEvent::ChannelMessage(own_event));
        Ok(())
    }

    /// `body` sealed under the channel's current text key, with that key's
    /// generation, which a receiver opens it under exactly.
    fn seal_edit(
        &self,
        community_id: &str,
        channel_id: &str,
        body: &str,
    ) -> Result<(Vec<u8>, u64), GovernanceRuntimeError> {
        let channel = ChannelId::from_hex(channel_id).ok_or_else(|| adapter("not a channel id"))?;
        let keys = crate::daemon::mek_rotation::key_provider(&self.ctx);
        let scope = keys.scope_for_text(community_id, channel);
        let (epoch, key) = rekindle_types::channel_keys::current_key(&*keys, community_id, scope)
            .ok_or_else(|| adapter("no text key for this channel"))?;
        let sealed =
            rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(*key, epoch.0)
                .encrypt(body.as_bytes())
                .map_err(|e| adapter(format!("seal edit: {e}")))?;
        Ok((sealed, epoch.0))
    }

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
