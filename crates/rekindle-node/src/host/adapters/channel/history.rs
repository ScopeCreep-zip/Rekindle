//! A channel's history on both modes (plan E3.1 E2c): our stored rows (the
//! outbox reader stores each post after the D26 gate, plan E3.3) with the
//! reactions and polls their authors' outboxes name.

use rekindle_channel::error::ChannelError;
use rekindle_channel::history;
use rekindle_channel::ChannelMessagingDeps;
use rekindle_db::repo::messages::channel::{self as rows, StoredChannelMessage};
use rekindle_db::repo::messages::channel_admission::{self, Placeholder};
use rekindle_types::dto::channel_message::{ChannelMessageView, MessageAttachment};
use rekindle_types::message::SendState;
use rekindle_types::permissions;

use super::{admission::admission_view, HostChannel};

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
        let attached = self.channel_attachments(community_id, channel_id).await;
        let (key, channel) = (owner.clone(), channel_id.to_string());
        let before_row = before.map(|b| i64::try_from(b).unwrap_or(i64::MAX));
        let community = community_id.to_owned();
        let (rows, placeholders) = db
            .call(move |conn| {
                Ok((
                    rows::page(conn, &key, &channel, before_row, limit)?,
                    channel_admission::page(conn, &key, &community, &channel, before_row, limit)?,
                ))
            })
            .await
            .map_err(stored)?;
        let mut messages: Vec<ChannelMessageView> = rows
            .into_iter()
            .map(|row| stored_view(row, &owner, &me))
            .collect();
        for view in &mut messages {
            self.attach_history(community_id, &attached, view);
        }
        Ok(with_placeholders(messages, placeholders, false, limit))
    }

    pub(super) async fn channel_attachments(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> history::ChannelAttachments {
        // For a removed community the read serves only the rows we already hold.
        if self.ctx().communities.own_removal(community_id).is_some() {
            history::ChannelAttachments::default()
        } else {
            history::read_attachments(self, community_id, channel_id).await
        }
    }

    pub(super) fn attach_history(
        &self,
        community_id: &str,
        attached: &history::ChannelAttachments,
        view: &mut ChannelMessageView,
    ) {
        view.forwarded_from_name =
            self.forward_name(community_id, view.forwarded_from_author.as_deref());
        let Some(id) = view.server_message_id.as_deref() else {
            return;
        };
        if view.reactions.is_none() {
            view.reactions = attached.reactions.get(id).cloned();
        }
        if view.poll.is_none() {
            view.poll = attached
                .polls
                .get(&(id.to_owned(), view.sender_id.clone()))
                .cloned();
        }
    }
}

pub(super) fn with_placeholders(
    mut messages: Vec<ChannelMessageView>,
    placeholders: Vec<Placeholder>,
    thread: bool,
    limit: u32,
) -> Vec<ChannelMessageView> {
    if !placeholders.is_empty() {
        let held: std::collections::HashSet<_> = messages
            .iter()
            .filter_map(|view| Some((view.sender_id.clone(), view.server_message_id.clone()?)))
            .collect();
        messages.extend(placeholders.into_iter().filter_map(|mut placeholder| {
            if thread {
                placeholder.message_id = rekindle_types::thread::thread_reply_key(
                    &placeholder.sender,
                    &placeholder.message_id,
                );
            }
            let key = (placeholder.sender.clone(), placeholder.message_id.clone());
            (!held.contains(&key)).then(|| placeholder_view(placeholder))
        }));
    }
    messages.sort_by_key(|message| message.timestamp);
    if messages.len() > limit as usize {
        messages.drain(..messages.len() - limit as usize);
    }
    messages
}

pub(super) fn placeholder_view(placeholder: Placeholder) -> ChannelMessageView {
    ChannelMessageView {
        id: 0,
        sender_id: placeholder.sender,
        body: String::new(),
        admission: Some(admission_view(placeholder.status)),
        decryption_failed: false,
        undecryptable: None,
        automod_blurred: false,
        timestamp: placeholder.timestamp,
        is_own: false,
        server_message_id: Some(placeholder.message_id),
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: None,
        forwarded_from_name: None,
        attachment: None,
        flags: 0,
        send_state: SendState::Delivered,
        failure_reason: None,
        edited_at: None,
        removed_by: None,
        history_gap_through: None,
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
        admission: None,
        id: row.id,
        sender_id: row.sender_key,
        body: row.body,
        decryption_failed: row.undecryptable_reason.is_some(),
        undecryptable: (row.undecryptable_reason.as_deref())
            .and_then(rekindle_types::dto::channel_message::UndecryptableReason::parse)
            .map(rekindle_types::dto::channel_message::Undecryptable::from),
        automod_blurred: row.automod_blurred,
        timestamp: row.timestamp,
        is_own,
        server_message_id: row.message_id,
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: row.forwarded_from_author,
        forwarded_from_name: None,
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
