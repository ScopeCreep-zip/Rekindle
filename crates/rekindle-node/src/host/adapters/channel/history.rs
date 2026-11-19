//! A channel's history on both modes (plan E3.1 E2c): our stored rows (the
//! outbox reader stores each post after the D26 gate, plan E3.3) with the
//! reactions and polls their authors' outboxes name.

use rekindle_channel::error::ChannelError;
use rekindle_channel::history;
use rekindle_channel::ChannelMessagingDeps;
use rekindle_db::repo::messages::channel::{self as rows, StoredChannelMessage};
use rekindle_types::dto::channel_message::{ChannelMessageView, MessageAttachment};
use rekindle_types::message::SendState;
use rekindle_types::permissions;

use super::HostChannel;

fn stored(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(format!("store: {e}"))
}

impl HostChannel {
    /// The latest `limit` messages of `channel_id` in `community_id`, only
    /// those older than `before` when given: our rows (the outbox reader
    /// stores every post through the D26 gate), oldest first. A community we were removed from answers our
    /// rows only.
    ///
    /// # Errors
    /// We may not read the channel's history, no identity is unlocked, or
    /// the store or the record read failed.
    pub(crate) async fn history(
        &self,
        community_id: &str,
        channel_id: &str,
        before: Option<u64>,
        limit: u32,
    ) -> Result<Vec<ChannelMessageView>, ChannelError> {
        self.require_channel_permission(community_id, Some(channel_id), permissions::READ_HISTORY)?;
        let (owner, db) = self.owner_and_db()?;
        let me = self.my_pseudonym_impl(community_id).unwrap_or_default();
        let removed = self.ctx().communities.own_removal(community_id).is_some();
        // A community we were removed from is read-only history (plan E3.2
        // S9): our rows as they were when we opened them, with nothing read
        // from the authors' outboxes again.
        let attached = if removed {
            history::ChannelAttachments::default()
        } else {
            history::read_attachments(self, community_id, channel_id).await
        };
        let (key, channel) = (owner.clone(), channel_id.to_string());
        let before_row = before.map(|b| i64::try_from(b).unwrap_or(i64::MAX));
        let rows = db
            .call(move |conn| rows::page(conn, &key, &channel, before_row, limit))
            .await
            .map_err(stored)?;
        let mut messages: Vec<ChannelMessageView> = rows
            .into_iter()
            .map(|row| stored_view(row, &owner, &me))
            .collect();
        // Each post's reactions and polls ride their authors' outboxes
        // (plan E3.3); a poll shows only on its creator's own message.
        let history::ChannelAttachments { reactions, polls } = attached;
        for view in &mut messages {
            let Some(id) = view.server_message_id.as_deref() else {
                continue;
            };
            if view.reactions.is_none() {
                view.reactions = reactions.get(id).cloned();
            }
            if view.poll.is_none() {
                view.poll = (polls.get(&(id.to_owned(), view.sender_id.clone()))).cloned();
            }
        }
        messages.sort_by_key(|message| message.timestamp);
        Ok(messages)
    }
}

/// One stored row as a frontend renders it; `own_key` and `pseudonym` mark
/// which senders are us.
pub(crate) fn stored_view(
    row: StoredChannelMessage,
    own_key: &str,
    pseudonym: &str,
) -> ChannelMessageView {
    let is_own = row.sender_key == own_key || row.sender_key == pseudonym;
    let attachment = (row.attachment_json.as_deref())
        .and_then(|json| serde_json::from_str::<rekindle_files::AttachmentRecordJson>(json).ok())
        .map(|record| MessageAttachment {
            downloaded: record.local_path.is_some(),
            attachment_id: record.attachment_id,
            filename: record.filename,
            mime_type: record.mime_type,
            total_size: record.total_size,
            chunk_count: record.chunk_count,
        });
    ChannelMessageView {
        id: row.id,
        sender_id: row.sender_key,
        body: row.body,
        decryption_failed: false,
        automod_blurred: row.automod_blurred,
        timestamp: row.timestamp,
        is_own,
        server_message_id: row.message_id,
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: row.forwarded_from_author,
        attachment,
        flags: row.flags,
        failure_reason: row
            .failure_reason
            .filter(|_| row.send_state == SendState::Failed),
        send_state: row.send_state,
        edited_at: row.edited_at,
        removed_by: row.removed_by,
        history_gap_through: row.history_gap_through,
    }
}
