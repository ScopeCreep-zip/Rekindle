//! Edits and deletes of channel messages on both modes (plan E3.3): an
//! author's change is an entry in its own record (the edit or delete kind of
//! `ChannelRecordEntry`), never a side message. The author stores it on its
//! own row and writes it to its slot; a reader applies it from every path
//! that reads the author's record (the gossip hint's slot read, the watched
//! change, the history read). Both judge it by `rekindle_channel::changes`
//! (E1–E3) and store it in the message's row, so it survives a reload.

use rekindle_channel::changes::{self, ChangeKind, ChangeRefusal, ChangeTarget};
use rekindle_channel::error::ChannelError;
use rekindle_channel::ingest::Verdict;
use rekindle_channel::send::{channel_message_subkey, SlotPosition};
use rekindle_channel::ChannelMessagingDeps;
use rekindle_codec::community::channel_record::{
    decode_channel_page, ChannelDelete, ChannelEdit, ChannelMessage, ChannelRecordEntry,
};
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_db::repo::messages::channel_changes::{self as change_rows, StoredEdit};
use rekindle_protocol::dht::community::channel_record as record;
use rekindle_records::port::RecordPort;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::ingest::{GateInputs, GatedPost, Ingested, ReceivedPost};
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

/// One author's slot value, read for a gossip hint: its signed author and
/// entries.
pub(crate) struct AuthorPage {
    pub author: PseudonymKey,
    pub entries: Vec<ChannelRecordEntry>,
}

/// Where a reader read an author's changes.
#[derive(Clone, Copy)]
pub(crate) struct ChangeSource<'a> {
    pub community_id: &'a str,
    pub channel_id: &'a str,
    pub record_key: &'a str,
    pub subkey_index: u32,
    pub author: &'a PseudonymKey,
}

impl HostChannel {
    /// Apply our `change` to our message `message_id` of `channel_id`: judged
    /// by E1–E3 first, written to our slot as an entry, then stored on our
    /// row and published to our own frontends (the mesh never echoes to its
    /// sender). One gossip hint names the message, so readers read our slot
    /// at once.
    ///
    /// # Errors
    /// We were removed, a rule refuses the change, no key seals the edit,
    /// the write could not be made, or the store failed.
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
        let seq = self.increment_lamport(community_id)?;
        let kind = match change {
            MessageChange::Edit { .. } => ChangeKind::Edit { seq },
            MessageChange::Delete => ChangeKind::Delete,
        };
        changes::judge_change(target.as_ref(), &me, kind).map_err(ChannelError::ChangeRefused)?;
        let context = self.channel_write_context(community_id, channel_id)?;
        let subkey_index = channel_message_subkey(context.slot_index);
        let now = self.now_secs();
        let (entry, body, digest) = match change {
            MessageChange::Edit { body } => {
                let at = SlotPosition {
                    channel_record_key: &context.channel_key,
                    subkey_index,
                    lamport_ts: seq,
                };
                let (ciphertext, mek_generation) =
                    changes::seal_edit(self, community_id, channel_id, at, &body)?;
                let digest = blake3::hash(&ciphertext).to_hex().to_string();
                let edit = ChannelEdit {
                    message_id: message_id.to_owned(),
                    ciphertext,
                    mek_generation,
                    edited_at: now,
                    lamport_ts: seq,
                };
                (ChannelRecordEntry::Edit(edit), body, digest)
            }
            MessageChange::Delete => {
                let delete = ChannelDelete {
                    message_id: message_id.to_owned(),
                    deleted_at: now,
                    lamport_ts: seq,
                };
                let digest = blake3::hash(format!("delete:{message_id}:{seq}").as_bytes());
                (
                    ChannelRecordEntry::Delete(delete),
                    String::new(),
                    digest.to_hex().to_string(),
                )
            }
        };
        let w = self.writer(&context)?;
        let written = record::write_member_change(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            entry.clone(),
        )
        .await;
        super::delivery(written, "message change write")?;
        let author = PseudonymKey::from_hex_lossy(&me);
        let source = ChangeSource {
            community_id,
            channel_id,
            record_key: &context.channel_key,
            subkey_index,
            author: &author,
        };
        self.store_change(&db, &owner, source, &entry, &body, false)
            .await?;
        let hint = CommunityEnvelope::MessageNotification {
            channel_id: channel_id.to_owned(),
            message_id: message_id.to_owned(),
            author_pseudonym: me,
            subkey_index,
            lamport_ts: seq,
            sequence: 0,
            content_hash: digest,
            timestamp: now.saturating_mul(1000),
        };
        if let Err(error) = self.send_to_mesh(community_id, &hint) {
            tracing::debug!(%error, "message change hint not sent");
        }
        Ok(())
    }

    /// Ingest the post a gossip hint named from `page` (when the page still
    /// holds it), then apply every change the page's author made.
    pub(crate) async fn ingest_page(
        &self,
        source: ChangeSource<'_>,
        sequence: u64,
        message_id: &str,
        page: AuthorPage,
    ) -> Result<Ingested, ChannelError> {
        let message = page.entries.iter().find_map(|entry| match entry {
            ChannelRecordEntry::Message(m) if m.message_id.as_deref() == Some(message_id) => {
                Some(m.clone())
            }
            _ => None,
        });
        let ingested = match message {
            Some(message) => {
                let post = ReceivedPost {
                    community_id: source.community_id.to_owned(),
                    channel_id: source.channel_id.to_owned(),
                    record_key: source.record_key.to_owned(),
                    subkey_index: source.subkey_index,
                    sequence,
                    message,
                    forwarded_from_author: None,
                };
                self.ingest(post).await?
            }
            None => Ingested::Duplicate,
        };
        self.apply_changes(source, &page.entries).await;
        Ok(ingested)
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
                let at = SlotPosition {
                    channel_record_key: source.record_key,
                    subkey_index: source.subkey_index,
                    lamport_ts: edit.lamport_ts,
                };
                let body = changes::open_edit(
                    self,
                    source.community_id,
                    source.channel_id,
                    edit.mek_generation,
                    at,
                    &edit.ciphertext,
                )
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

/// `author`'s slot value at `subkey_index` of `record_key`, when it holds
/// the post `message_id` or a change of it (a gossip hint names the post).
///
/// # Errors
/// The read failed, the slot is empty or fails verification, another
/// author wrote it, or nothing in it names the message.
pub(crate) async fn find_page(
    pool: &dyn RecordPort,
    record_key: &str,
    subkey_index: u32,
    message_id: &str,
    author: &str,
) -> Result<AuthorPage, String> {
    let value = (pool.read_once(record_key, subkey_index, true).await)
        .map_err(|e| format!("{record_key}: {e}"))?
        .ok_or_else(|| format!("{record_key}: slot is empty"))?;
    let (page_author, entries) =
        decode_channel_page(&value.data).map_err(|e| format!("{record_key}: {e}"))?;
    if !hex::encode(page_author.0).eq_ignore_ascii_case(author) {
        return Err(format!("{record_key}: the slot is another author's"));
    }
    let names = |entry: &ChannelRecordEntry| match entry {
        ChannelRecordEntry::Message(ChannelMessage {
            message_id: Some(id),
            ..
        }) => id == message_id,
        ChannelRecordEntry::Edit(edit) => edit.message_id == message_id,
        ChannelRecordEntry::Delete(delete) => delete.message_id == message_id,
        _ => false,
    };
    if !entries.iter().any(names) {
        return Err(format!("{record_key}: no message {message_id}"));
    }
    Ok(AuthorPage {
        author: page_author,
        entries,
    })
}
