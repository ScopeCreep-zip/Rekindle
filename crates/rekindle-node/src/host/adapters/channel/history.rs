//! A channel's history on both modes (plan E3.1 E2c, from the desktop's
//! `messaging_runtime` and `channel_materialize`): our stored rows merged
//! with the channel record's messages, of which those we did not hold and
//! could open pass the D26 gate before they are stored or shown.

use rekindle_channel::error::ChannelError;
use rekindle_channel::history::{self, RecordMessage};
use rekindle_channel::ingest::{BatchWindows, GateReason, Verdict};
use rekindle_channel::ChannelMessagingDeps;
use rekindle_db::repo::messages::channel::{
    self as rows, ChannelMessageInsert, StoredChannelMessage,
};
use rekindle_types::dto::channel_message::{ChannelMessageView, MessageAttachment};
use rekindle_types::message::SendState;
use rekindle_types::permissions;

use super::ingest::GatedPost;
use super::HostChannel;

fn stored(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(format!("store: {e}"))
}

impl HostChannel {
    /// The latest `limit` messages of `channel_id` in `community_id`, only
    /// those older than `before` when given: our rows and the record's,
    /// merged, oldest first.
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
        let record =
            history::read_channel_record(self, community_id, channel_id, before, limit).await?;
        let incoming = (self.store_record_messages(&owner, community_id, channel_id, record))
            .await?
            .into_iter()
            .map(|message| message.view)
            .collect();
        history::merge_message_lists(&mut messages, incoming, limit);
        Ok(messages)
    }

    /// Gate (D26) and store each record message we do not hold yet, and
    /// answer the record's messages as they may be shown. A post the gate
    /// drops is neither stored nor shown; one it hides is stored and shown
    /// blurred; one we hold carries its stored row's verdict. A message we
    /// could not open is not judged or stored (its row would keep an empty
    /// body after its key arrives) and shows only as undecryptable. Posts
    /// are judged in the record's channel order.
    ///
    /// # Errors
    /// No identity is unlocked, the community's governance state is not
    /// loaded, or the store failed.
    pub(crate) async fn store_record_messages(
        &self,
        owner: &str,
        community_id: &str,
        channel_id: &str,
        record: Vec<RecordMessage>,
    ) -> Result<Vec<RecordMessage>, ChannelError> {
        let (_, db) = self.owner_and_db()?;
        let (key, channel) = (owner.to_string(), channel_id.to_string());
        let ids: Vec<Option<String>> = (record.iter())
            .map(|m| m.view.server_message_id.clone())
            .collect();
        let held = db
            .call(move |conn| {
                (ids.iter())
                    .map(|id| match id {
                        Some(id) => rows::held_blurred(conn, &key, &channel, id),
                        None => Ok(None),
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(stored)?;
        let judged =
            (record.iter().zip(&held)).any(|(m, h)| h.is_none() && !m.view.decryption_failed);
        let inputs = if judged {
            Some(self.gate_inputs(owner, &db, community_id).await?)
        } else {
            None
        };
        let (mut shown, mut fresh, mut reports) = (Vec::new(), Vec::new(), Vec::new());
        // The posts this page admits or hides are stored after the loop,
        // so a later post of the page is judged against them here.
        let mut batch = BatchWindows::default();
        for (mut message, held) in record.into_iter().zip(held) {
            if let Some(blurred) = held {
                message.view.automod_blurred = blurred;
                shown.push(message);
                continue;
            }
            let Some(inputs) = inputs.as_ref().filter(|_| !message.view.decryption_failed) else {
                shown.push(message);
                continue;
            };
            let timestamp_ms = u64::try_from(message.view.timestamp).unwrap_or(0);
            let sender = message.view.sender_id.as_str();
            let stored = self
                .stored_before_secs(
                    owner,
                    &db,
                    channel_id,
                    sender,
                    timestamp_ms,
                    message.view.server_message_id.as_deref(),
                )
                .await?;
            let at_secs = timestamp_ms / 1000;
            let verdict = self.verdict(
                inputs,
                &GatedPost {
                    community_id,
                    channel_id,
                    sender_pseudonym: sender,
                    body: &message.view.body,
                    timestamp_ms,
                    held_before_secs: batch.held_before(sender, at_secs, stored),
                    backfill: true,
                },
            );
            batch.record(sender, at_secs, &verdict);
            let report = match verdict {
                Verdict::Drop(reason) => {
                    tracing::debug!(community_id, channel_id, ?reason, "backfilled post dropped");
                    continue;
                }
                Verdict::Hide(_) => {
                    message.view.automod_blurred = true;
                    None
                }
                Verdict::Report(GateReason::AutoMod { rule }) => Some(rule),
                Verdict::Report(reason) => Some(format!("{reason:?}")),
                Verdict::Admit => None,
            };
            if let Some(id) = message.view.server_message_id.clone() {
                if let Some(reason) = report {
                    reports.push((id, reason));
                }
                fresh.push(message.clone());
            }
            shown.push(message);
        }
        self.insert_record_messages(owner, channel_id, fresh)
            .await?;
        for (message_id, reason) in reports {
            self.report_to_moderators(community_id, channel_id, &message_id, &reason)
                .await;
        }
        Ok(shown)
    }

    /// Store the gated record messages `fresh`, each with its verdict's
    /// blur; one stored meanwhile is skipped.
    async fn insert_record_messages(
        &self,
        owner: &str,
        channel_id: &str,
        fresh: Vec<RecordMessage>,
    ) -> Result<(), ChannelError> {
        let (_, db) = self.owner_and_db()?;
        let (owner, channel) = (owner.to_string(), channel_id.to_string());
        let attachments = fresh
            .iter()
            .map(|m| {
                (m.attachment.as_ref())
                    .map(|offer| {
                        serde_json::to_string(&rekindle_files::AttachmentRecordJson::from_offer(
                            offer, None,
                        ))
                    })
                    .transpose()
                    .map_err(|e| ChannelError::Adapter(format!("attachment record: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        db.call(move |conn| {
            for (message, attachment_json) in fresh.iter().zip(&attachments) {
                let Some(message_id) = message.view.server_message_id.as_deref() else {
                    continue;
                };
                if rows::row_id_by_message_id(conn, &owner, &channel, message_id)?.is_some() {
                    continue;
                }
                rows::insert(
                    conn,
                    &ChannelMessageInsert {
                        reply_to: message.reply_to.as_deref(),
                        owner_key: &owner,
                        channel_id: &channel,
                        sender_key: &message.view.sender_id,
                        body: &message.view.body,
                        timestamp: message.view.timestamp,
                        is_read: true,
                        mek_generation: Some(
                            i64::try_from(message.mek_generation).unwrap_or(i64::MAX),
                        ),
                        message_id,
                        lamport_ts: message.lamport_ts,
                        automod_blurred: message.view.automod_blurred,
                        forwarded_from_author: None,
                        flags: 0,
                        attachment_json: attachment_json.as_deref(),
                        send_state: SendState::Delivered,
                        mentions_me: message.mentions_me,
                    },
                )?;
            }
            Ok(())
        })
        .await
        .map_err(stored)
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
        send_state: row.send_state,
    }
}
