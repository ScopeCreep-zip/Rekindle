//! The attachment rows (`rekindle_db::repo::messages`): our attachment
//! message, the record a download, playback or reveal names, and the place
//! a download was saved to.

use std::path::Path;

use rekindle_db::repo::messages::{attachments, channel};
use rekindle_files::{AttachmentRecordJson, FilesError, InsertChannelMessage};
use rekindle_types::message::SendState;

use super::HostFiles;

fn stored(e: impl std::fmt::Display) -> FilesError {
    FilesError::Db(e.to_string())
}

impl HostFiles {
    /// Store our attachment message, `queued` until its outbox row settles
    /// (plan E2.5).
    pub(super) async fn insert_row(&self, row: InsertChannelMessage<'_>) -> Result<(), FilesError> {
        let mek_generation = i64::try_from(row.mek_generation).unwrap_or(i64::MAX);
        let (owner, chan, sender) = (
            row.owner_key.to_string(),
            row.channel_id.to_string(),
            row.sender_key.to_string(),
        );
        let (mid, attachment_json, body) = (
            row.message_id.to_string(),
            row.attachment_json.to_string(),
            row.body.to_string(),
        );
        let (timestamp, lamport_ts, flags) = (row.timestamp_ms, row.lamport_ts, row.flags);
        (self.db()?)
            .call(move |conn| {
                channel::insert(
                    conn,
                    &channel::ChannelMessageInsert {
                        reply_to: None,
                        owner_key: &owner,
                        channel_id: &chan,
                        sender_key: &sender,
                        body: &body,
                        timestamp,
                        is_read: true,
                        mek_generation: Some(mek_generation),
                        message_id: &mid,
                        lamport_ts,
                        automod_blurred: false,
                        forwarded_from_author: None,
                        flags,
                        attachment_json: Some(&attachment_json),
                        send_state: SendState::Queued,
                        mentions_me: false,
                    },
                )
            })
            .await
            .map_err(|e| stored(format!("insert attachment row: {e}")))
    }

    /// The stored record of `attachment_id_hex` in `channel_id`, from the
    /// message row that carries it.
    pub async fn record(
        &self,
        channel_id: &str,
        attachment_id_hex: &str,
    ) -> Result<Option<(String, AttachmentRecordJson)>, FilesError> {
        use rekindle_files::FilesDeps as _;
        let owner = self.owner_key()?;
        let chan = channel_id.to_string();
        let rows = (self.db()?)
            .call(move |conn| attachments::channel_records(conn, &owner, &chan))
            .await
            .map_err(stored)?;
        for (message_id, json) in rows {
            let Some(message_id) = message_id else {
                continue;
            };
            let record: AttachmentRecordJson = serde_json::from_str(&json).map_err(stored)?;
            if record.attachment_id == attachment_id_hex {
                return Ok(Some((message_id, record)));
            }
        }
        Ok(None)
    }

    /// Store where a download of `attachment_id_hex` was saved.
    pub(super) async fn store_local_path(
        &self,
        owner_key: &str,
        channel_id: &str,
        attachment_id_hex: &str,
        save_path: &Path,
    ) -> Result<(), FilesError> {
        let Some((message_id, mut record)) = self.record(channel_id, attachment_id_hex).await?
        else {
            return Err(FilesError::NotFound(format!(
                "attachment {attachment_id_hex} in {channel_id}"
            )));
        };
        record.local_path = Some(save_path.display().to_string());
        let json = serde_json::to_string(&record).map_err(stored)?;
        let (owner, chan) = (owner_key.to_string(), channel_id.to_string());
        (self.db()?)
            .call(move |conn| attachments::set_record(conn, &owner, &chan, &message_id, &json))
            .await
            .map_err(|e| stored(format!("store attachment local_path: {e}")))
    }
}
