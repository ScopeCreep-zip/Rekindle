//! Edits and deletes of channel messages on both modes (plan E3.3): an
//! author's change is an entry in its own record (the edit or delete kind of
//! `ChannelRecordEntry`), never a side message. The author stores it on its
//! own row and queues it on its channel outbox (`M`/4); a reader applies it
//! from the outbox read. Both judge it by `rekindle_channel::changes`
//! (E1–E3) and store it in the message's row, so it survives a reload.

use rekindle_channel::changes::{self, ChangeKind, ChangeRefusal, ChangeTarget};
use rekindle_channel::error::ChannelError;
use rekindle_channel::ingest::Verdict;
use rekindle_channel::outbox_seal::open_entry_body;
use rekindle_channel::send::BodyPosition;
use rekindle_channel::ChannelMessagingDeps;
use rekindle_codec::community::channel_record::{ChannelDelete, ChannelEdit, ChannelRecordEntry};
use rekindle_db::repo::messages::channel_changes::{self as change_rows, StoredEdit};
use rekindle_types::id::PseudonymKey;
use rekindle_types::message::{ChannelPost, PostEntry};
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::ingest::{GateInputs, GatedPost};
use super::HostChannel;

fn stored(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(format!("store: {e}"))
}

/// What our change does to our message.
#[derive(Debug, Clone)]
pub(crate) enum MessageChange {
    /// Its body becomes `body`.
    Edit { body: String },
    /// It is deleted for members (E4).
    Delete,
}

/// Where a reader read an author's changes.
#[derive(Clone, Copy)]
pub(crate) struct ChangeSource<'a> {
    pub community_id: &'a str,
    pub channel_id: &'a str,
    /// The entry's stream seq in the author's channel outbox (`M`/4).
    pub stream_seq: u64,
    pub author: &'a PseudonymKey,
}

impl HostChannel {
    /// Apply our `change` to our message `message_id` of `channel_id`: judged
    /// by E1–E3 first, queued on the channel's `channel_post` stream as an
    /// entry of our outbox (`M`/4, plan E3.3; its body sealed when the
    /// stream drains it), then stored on our row and published to our own
    /// frontends. The drain's own subkey-4 hint tells readers to read it.
    ///
    /// # Errors
    /// We were removed, a rule refuses the change, we hold no outbox in the
    /// community, or the queue or the store failed.
    pub(crate) async fn change_own_message(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        change: MessageChange,
    ) -> Result<(), ChannelError> {
        self.refuse_removed(community_id)?;
        let me = (self.my_pseudonym_hex(community_id))
            .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
        let (owner, db) = self.owner_and_db()?;
        let target = self
            .change_target(&db, &owner, channel_id, message_id)
            .await?;
        let edit = matches!(change, MessageChange::Edit { .. });
        changes::judge_own_change(target.as_ref(), &me, edit)
            .map_err(ChannelError::ChangeRefused)?;
        // Refused when we hold no outbox in the community.
        self.gov
            .own_outbox(community_id)
            .map_err(ChannelError::Adapter)?;
        let now = self.now_secs();
        let target = message_id.to_owned();
        let nonce = hex::encode(rekindle_utils::random::id_bytes_16_from(
            &*rekindle_governance_runtime::deps::GovernanceRuntimeDeps::rng(&self.gov),
        ));
        let (queued, body, row) = match &change {
            MessageChange::Edit { body } => (
                PostEntry::Edit {
                    target: target.clone(),
                    body: body.clone(),
                    edited_at: now,
                },
                body.clone(),
                format!("edit:{target}:{nonce}"),
            ),
            MessageChange::Delete => (
                PostEntry::Delete {
                    target: target.clone(),
                    deleted_at: now,
                },
                String::new(),
                format!("delete:{target}:{nonce}"),
            ),
        };
        let post = ChannelPost {
            community: community_id.to_owned(),
            channel: channel_id.to_owned(),
            message: row,
            entry: queued,
        };
        // The change's place, and so its seq (E1), is its stream seq (D22).
        let seq = self.queue_change_impl(&post).await?;
        let entry = match change {
            MessageChange::Edit { .. } => ChannelRecordEntry::Edit(ChannelEdit {
                message_id: target,
                ciphertext: Vec::new(),
                mek_generation: 0,
                edited_at: now,
                lamport_ts: seq,
            }),
            MessageChange::Delete => ChannelRecordEntry::Delete(ChannelDelete {
                message_id: target,
                deleted_at: now,
                lamport_ts: seq,
            }),
        };
        let author = PseudonymKey::from_hex_lossy(&me);
        let source = ChangeSource {
            community_id,
            channel_id,
            stream_seq: seq,
            author: &author,
        };
        self.store_change(&db, &owner, source, &entry, &body, false)
            .await
    }

    /// Apply each edit and delete among `entries`, all by `source.author`, to
    /// the messages we hold: deletes first (a delete wins over any edit),
    /// then edits in the author's seq order, each judged by E1–E3 and an
    /// edit's new body by the D26 gate. Every change a row takes is
    /// published; a refused or repeated one changes nothing.
    pub(crate) async fn apply_changes(
        &self,
        source: ChangeSource<'_>,
        entries: &[ChannelRecordEntry],
    ) {
        let mut ordered: Vec<ChannelRecordEntry> =
            (entries.iter()).filter(|e| is_change(e)).cloned().collect();
        if ordered.is_empty() || self.refuse_removed(source.community_id).is_err() {
            return;
        }
        ordered.sort_by_key(change_order);
        let Ok((owner, db)) = self.owner_and_db() else {
            return;
        };
        let inputs = match self.gate_inputs(&owner, &db, source.community_id).await {
            Ok(inputs) => inputs,
            Err(error) => {
                tracing::debug!(%error, "message changes not judged");
                return;
            }
        };
        for entry in &ordered {
            if let Err(error) = self.apply_change(&db, &owner, &inputs, source, entry).await {
                tracing::debug!(%error, "message change not applied");
            }
        }
    }

    pub(super) async fn apply_change(
        &self,
        db: &rekindle_db::Db,
        owner: &str,
        inputs: &GateInputs,
        source: ChangeSource<'_>,
        entry: &ChannelRecordEntry,
    ) -> Result<(), ChannelError> {
        let author = hex::encode(source.author.0);
        let (message_id, kind) = match entry {
            ChannelRecordEntry::Edit(e) => (&e.message_id, ChangeKind::Edit { seq: e.lamport_ts }),
            ChannelRecordEntry::Delete(d) => (&d.message_id, ChangeKind::Delete),
            _ => return Ok(()),
        };
        let target = (self.change_target(db, owner, source.channel_id, message_id)).await?;
        if let Err(refusal) = changes::judge_change(target.as_ref(), &author, kind) {
            if refusal == ChangeRefusal::NoTarget {
                self.hold_change(source, entry);
            }
            return Err(ChannelError::ChangeRefused(refusal));
        }
        let (body, blurred) = match entry {
            ChannelRecordEntry::Edit(edit) => {
                let body = self
                    .open_change_body(source, edit)
                    .ok_or_else(|| ChannelError::Decrypt("edit body".into()))?;
                // The new body passes the gate as a post would: a refused
                // author's edit, or one automod blocks, is not stored.
                let gated = GatedPost {
                    community_id: source.community_id,
                    channel_id: source.channel_id,
                    sender_pseudonym: &author,
                    body: &body,
                    timestamp_ms: edit.edited_at.saturating_mul(1000),
                    held_before_secs: None,
                    backfill: false,
                    slot: None,
                };
                let blurred = match self.verdict(inputs, &gated) {
                    Verdict::Drop(reason) => {
                        return Err(ChannelError::Adapter(format!("edit dropped: {reason:?}")))
                    }
                    Verdict::Hide(_) => true,
                    Verdict::Admit | Verdict::Report(_) => false,
                };
                (body, blurred)
            }
            _ => (String::new(), false),
        };
        self.store_change(db, owner, source, entry, &body, blurred)
            .await
    }

    /// An edit's new body, opened at its own position: its outbox entry's
    /// `{anchor, channel, author, streamSeq}`.
    fn open_change_body(&self, source: ChangeSource<'_>, edit: &ChannelEdit) -> Option<String> {
        let channel = rekindle_types::id::ChannelId::from_hex(source.channel_id)?;
        let scope = self.keys().scope_for_text(source.community_id, channel);
        let at = BodyPosition {
            anchor: source.community_id,
            channel_id: source.channel_id,
            author: &source.author.0,
            stream_seq: source.stream_seq,
        };
        let keys = self.keys();
        open_entry_body(
            &*keys,
            source.community_id,
            scope,
            edit.mek_generation,
            at,
            &edit.ciphertext,
        )
        .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Store `entry` (its rules re-checked in SQL) and publish the change
    /// when the row took it.
    async fn store_change(
        &self,
        db: &rekindle_db::Db,
        owner: &str,
        source: ChangeSource<'_>,
        entry: &ChannelRecordEntry,
        body: &str,
        blurred: bool,
    ) -> Result<(), ChannelError> {
        let (owner, channel) = (owner.to_owned(), source.channel_id.to_owned());
        let (author, body, entry) = (hex::encode(source.author.0), body.to_owned(), entry.clone());
        let (event_entry, event_body) = (entry.clone(), body.clone());
        let changed = db
            .call(move |conn| match &entry {
                ChannelRecordEntry::Edit(edit) => change_rows::store_edit(
                    conn,
                    &StoredEdit {
                        owner_key: &owner,
                        channel_id: &channel,
                        message_id: &edit.message_id,
                        author: &author,
                        body: &body,
                        edited_at: edit.edited_at,
                        seq: edit.lamport_ts,
                        blurred,
                    },
                ),
                ChannelRecordEntry::Delete(delete) => change_rows::tombstone(
                    conn,
                    &owner,
                    &channel,
                    &delete.message_id,
                    &author,
                    delete.deleted_at,
                ),
                _ => Ok(false),
            })
            .await
            .map_err(stored)?;
        if !changed {
            return Ok(());
        }
        let (community, channel) = (source.community_id.to_owned(), source.channel_id.to_owned());
        let event = match event_entry {
            ChannelRecordEntry::Edit(edit) => ChannelMessageEvent::Edited {
                community,
                channel,
                message_id: edit.message_id,
                edited_at: edit.edited_at,
                body: Some(event_body),
            },
            ChannelRecordEntry::Delete(delete) => ChannelMessageEvent::Deleted {
                community,
                channel,
                message_id: delete.message_id,
                removed_by: None,
            },
            _ => return Ok(()),
        };
        self.ctx()
            .publish_event(SubscriptionEvent::ChannelMessage(event));
        Ok(())
    }

    async fn change_target(
        &self,
        db: &rekindle_db::Db,
        owner: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Result<Option<ChangeTarget>, ChannelError> {
        let (owner, channel, message) = (
            owner.to_owned(),
            channel_id.to_owned(),
            message_id.to_owned(),
        );
        let row = db
            .call(move |conn| change_rows::change_target(conn, &owner, &channel, &message))
            .await
            .map_err(stored)?;
        Ok(row.map(|row| ChangeTarget {
            sender_key: row.sender_key,
            edit_seq: row.edit_seq,
            edit_count: row.edit_count,
            deleted: row.deleted,
        }))
    }
}

fn is_change(entry: &ChannelRecordEntry) -> bool {
    matches!(
        entry,
        ChannelRecordEntry::Edit(_) | ChannelRecordEntry::Delete(_)
    )
}

/// Deletes first (a delete wins over any edit), then by the author's seq.
fn change_order(entry: &ChannelRecordEntry) -> (bool, u64) {
    (
        !matches!(entry, ChannelRecordEntry::Delete(_)),
        entry.lamport(),
    )
}
