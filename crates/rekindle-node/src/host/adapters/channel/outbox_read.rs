//! The channel outbox reader (plan E3.3, D23): each author's `M`/4 chain is
//! one stream `channel_outbox:<community>` whose writer is the author's
//! pseudonym, diffed by the D23 engine
//! ([`rekindle_sync::stream::StreamEngine::diff_log`]) from the
//! `stream_cursors` cursor: `cursor + 1 == seq` runs on the author's chain
//! seq, so the read is gap-free. Each entry is verified under its author
//! at its chain seq, assigned to the channel whose tag it carries under its
//! generation ([`rekindle_channel::outbox_seal::stream_of_tag`]), and its
//! post goes through the one ingest path (D26), its body opened at
//! `{anchor, channel, author, streamSeq}`. An entry we cannot verify or
//! link, or whose body no key we hold opens, still advances the cursor:
//! the chain covers ciphertext. The one exception is a joiner before its
//! first community key: no entry's tag maps yet, so the cursor stays and
//! the key's install catches every outbox up again.
//!
//! The cursor's writer is the author at its member record: a member that
//! left and re-joined writes a fresh record whose chain starts at 1 again.
//!
//! A departed author's entry is kept only at or below a head its
//! departures recorded of this outbox (`GovernanceState::departed_heads`
//! under its member record), and only when the entry at that head in the
//! chain we read has the head's digest; one appended after the departure,
//! or into a gap below a head signed above the published chain, is refused
//! by the D26 gate as from a non-member (A/B R3-8, plan S9 carried onto
//! the outbox).
//!
//! A departure's head is a head the reader knows exists: a read that stops
//! below it (the closest replicas agree on an older head the departed
//! writer can no longer repair) runs bounded gap reads through a DHT
//! transaction ([`rekindle_records::chain::read_log_reaching`]); one that
//! still falls short leaves the cursor below the head and is not caught up.
//!
//! Triggers: the author's or a relayer's `HeadAdvanced` for subkey 4 and a
//! ring change of subkey 4 diff that author; a catch-up (a community's
//! load: an unlock, a join, a community key installed) diffs every member. Older history stays on the channel records
//! until E3.3 slice 8.

use async_trait::async_trait;
use rekindle_channel::outbox_seal::stream_of_tag;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_codec::member_record::{decode_outbox_entry, entry_hash, links_to, ChainPosition};
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::{SinkError, StreamSink};
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::id::PseudonymKey;

use crate::host::adapters::governance::slot_heads::outbox_head_digest;

use super::ingest::{Ingested, ReceivedAt, ReceivedPost};
use super::received::OutboxHead;
use super::HostChannel;

/// The stream of every author's channel outbox in `community_id`.
#[must_use]
pub fn outbox_stream(community_id: &str) -> String {
    format!("channel_outbox:{community_id}")
}

fn sink_error(e: impl std::fmt::Display) -> SinkError {
    SinkError(e.to_string())
}

/// One author's outbox as the D23 engine applies it.
struct OutboxSink<'a> {
    channel: &'a HostChannel,
    community_id: &'a str,
    author: [u8; 32],
    /// The author's member record.
    member_record: &'a str,
    /// Read by a catch-up rather than a live trigger.
    backfill: bool,
    /// Whether the author is a member, or which departure head keeps its
    /// entries.
    standing: Standing,
    /// The chain the entries are read from (the link check's reads).
    port: &'a dyn rekindle_records::port::RecordPort,
    /// The seq and stored-bytes hash of the entry this diff applied last.
    last: parking_lot::Mutex<Option<(u64, [u8; 32])>>,
}

#[async_trait]
impl StreamSink for OutboxSink<'_> {
    async fn cursor(&self, stream: &str, writer: &str) -> Result<u64, SinkError> {
        let (owner, db) = self.channel.owner_and_db().map_err(sink_error)?;
        let (stream, writer) = (stream.to_owned(), writer.to_owned());
        db.call(move |conn| rekindle_db::repo::stream_cursors::get(conn, &owner, &stream, &writer))
            .await
            .map_err(sink_error)
    }

    async fn apply(&self, stream: &str, writer: &str, entry: &ChainEntry) -> Result<(), SinkError> {
        let departure_head = match &self.standing {
            Standing::Departed(Some(head)) if entry.seq <= u64::from(head.seq) => {
                Some(head.clone())
            }
            _ => None,
        };
        let read = OutboxRead {
            community_id: self.community_id,
            author: &self.author,
            member_record: self.member_record,
            backfill: self.backfill,
            departed: matches!(self.standing, Standing::Departed(_)),
            departure_head,
            before: self.before(entry.seq).await,
        };
        self.channel
            .apply_outbox_entry(&read, entry)
            .await
            .map_err(sink_error)?;
        *self.last.lock() = Some((entry.seq, entry_hash(&entry.ciphertext)));
        self.advance(stream, writer, entry.seq).await
    }

    /// An outbox is never retired, so its read reports no gap.
    async fn history_gap(&self, stream: &str, writer: &str, through: u64) -> Result<(), SinkError> {
        tracing::warn!(
            stream,
            writer,
            through,
            "channel outbox reported a retired gap"
        );
        self.advance(stream, writer, through).await
    }

    async fn caught_up(&self, stream: &str, writer: &str) {
        tracing::debug!(stream, writer, "channel outbox caught up");
    }
}

impl OutboxSink<'_> {
    /// The entry before `seq` in the chain, as its link must name it:
    /// none for the first, the hash of the entry this diff applied before
    /// it, or else of the entry read from the chain.
    async fn before(&self, seq: u64) -> Before {
        if seq <= 1 {
            return Before::Known(None);
        }
        let wanted = seq - 1;
        if let Some((at, hash)) = *self.last.lock() {
            if at == wanted {
                return Before::Known(Some(hash));
            }
        }
        let read = rekindle_records::chain::read_log_reaching(
            self.port,
            self.member_record,
            OUTBOX,
            wanted.saturating_sub(1),
            wanted,
        )
        .await;
        (read.ok())
            .and_then(|read| read.entries.into_iter().find(|e| e.seq == wanted))
            .map_or(Before::Unread, |e| {
                Before::Known(Some(entry_hash(&e.ciphertext)))
            })
    }

    async fn advance(&self, stream: &str, writer: &str, seq: u64) -> Result<(), SinkError> {
        let (owner, db) = self.channel.owner_and_db().map_err(sink_error)?;
        let now = self.channel.ctx().clock.now_ms_i64();
        let (stream, writer) = (stream.to_owned(), writer.to_owned());
        db.call(move |conn| {
            rekindle_db::repo::stream_cursors::advance(conn, &owner, &stream, &writer, seq, now)
        })
        .await
        .map(drop)
        .map_err(sink_error)
    }
}

/// An outbox author's standing in the merged state.
enum Standing {
    Member,
    /// Departed: the highest of its departure heads of this outbox that
    /// the chain reproduces (`None`: none does, so none of its entries is
    /// kept).
    Departed(Option<OutboxHead>),
}

/// How one entry of an author's outbox was read.
struct OutboxRead<'a> {
    community_id: &'a str,
    author: &'a [u8; 32],
    /// The author's member record (the chain's record).
    member_record: &'a str,
    backfill: bool,
    /// Whether the author has departed.
    departed: bool,
    departure_head: Option<OutboxHead>,
    /// What the entry's `prev` must name ([`OutboxSink::before`]).
    before: Before,
}

/// The entry before one being read, as its `prev` must name it.
#[derive(Debug, Clone, Copy)]
enum Before {
    /// The hash of its stored bytes, `None` for the chain's first entry.
    Known(Option<[u8; 32]>),
    /// Its read failed, so the link is not checked.
    Unread,
}

impl HostChannel {
    /// Diff `author`'s channel outbox (`member_record`/4) in `community_id`
    /// from our cursor; `backfill` when a catch-up reads it.
    pub(crate) async fn read_author_outbox(
        &self,
        community_id: &str,
        author: &PseudonymKey,
        member_record: &str,
        backfill: bool,
    ) {
        if self.ctx().communities.own_removal(community_id).is_some() {
            return;
        }
        let engine = {
            let map = self.ctx().communities.map().read();
            map.get(community_id).map(|c| c.outbox_streams.clone())
        };
        let (Some(engine), Ok(port)) = (engine, self.records()) else {
            return;
        };
        let standing = self
            .standing(&*port, community_id, author, member_record)
            .await;
        let known_head = self.known_head(community_id, author, member_record);
        let sink = OutboxSink {
            channel: self,
            community_id,
            author: author.0,
            member_record,
            backfill,
            standing,
            port: &*port,
            last: parking_lot::Mutex::new(None),
        };
        let stream = outbox_stream(community_id);
        let writer = format!("{}:{member_record}", hex::encode(author.0));
        match engine
            .diff_log(
                &*port,
                &sink,
                (&stream, &writer),
                member_record,
                (OUTBOX, known_head),
            )
            .await
        {
            Ok(diffed) if diffed.unreached.is_some() => {
                tracing::warn!(community_id, author = %writer, head = ?diffed.unreached, applied = diffed.applied, "channel outbox head a departure recorded is on no replica we reached: the history keeps the gap");
            }
            Ok(diffed) if diffed.applied > 0 => {
                tracing::info!(community_id, author = %writer, applied = diffed.applied, "channel outbox read");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(community_id, author = %writer, %error, "channel outbox not read yet");
            }
        }
    }

    /// Catch up every member's channel outbox in `community_id` (a load of
    /// the community; plan E3.3 "Cursor and authors"), and each departed
    /// author's whose departures recorded a head of it, so a member joining
    /// later keeps what the author posted before it departed.
    pub(crate) async fn catch_up_outboxes(&self, community_id: &str) {
        let Some(state) = self.governance_state(community_id) else {
            return;
        };
        let departed = (state.departed_heads.iter()).filter_map(|(author, heads)| {
            let record = state.op_log_records.get(author)?;
            heads.contains_key(record).then_some((author, record))
        });
        let members =
            (state.members.iter()).map(|(author, member)| (author, &member.member_record));
        // In author order: the merged state's maps iterate in an order of
        // their own per instance, and each read's DHT calls go out in the
        // order the reads run (C9.2, one seed one trace).
        let mut authors: Vec<_> = members.chain(departed).collect();
        authors.sort_unstable_by_key(|(author, _)| author.0);
        for (author, record) in authors {
            self.read_author_outbox(community_id, author, record, true)
                .await;
        }
    }

    /// The highest head of `member_record`'s outbox a departure of
    /// `author` recorded (0: none), which the read must reach (D23).
    fn known_head(&self, community_id: &str, author: &PseudonymKey, member_record: &str) -> u64 {
        let Some(state) = self.governance_state(community_id) else {
            return 0;
        };
        (state.departed_heads.get(author))
            .and_then(|heads| heads.get(member_record))
            .and_then(|heads| heads.iter().next_back())
            .map_or(0, |(seq, _)| u64::from(*seq))
    }

    /// `author`'s standing: a member, or departed with the highest head its
    /// departures recorded of `member_record`'s outbox whose entry in the
    /// chain has the head's digest (none when none does, so the reader
    /// keeps none of its entries).
    async fn standing(
        &self,
        port: &dyn rekindle_records::port::RecordPort,
        community_id: &str,
        author: &PseudonymKey,
        member_record: &str,
    ) -> Standing {
        let Some(state) = self.governance_state(community_id) else {
            return Standing::Member;
        };
        if !state.is_departed(author) {
            return Standing::Member;
        }
        let heads = (state.departed_heads.get(author))
            .and_then(|heads| heads.get(member_record).cloned())
            .unwrap_or_default();
        for (seq, digest) in heads.into_iter().rev() {
            let wanted = u64::from(seq);
            // A head the closest replicas have not caught up to is read
            // from the replicas that hold it (D23 gap read).
            let read = rekindle_records::chain::read_log_reaching(
                port,
                member_record,
                OUTBOX,
                wanted.saturating_sub(1),
                wanted,
            )
            .await;
            let at_head =
                (read.ok()).and_then(|read| read.entries.into_iter().find(|e| e.seq == wanted));
            let reproduced = at_head.is_some_and(|entry| {
                outbox_head_digest(member_record, seq, &entry.ciphertext) == digest
            });
            if reproduced {
                return Standing::Departed(Some(OutboxHead {
                    member_record: member_record.to_owned(),
                    seq,
                    digest,
                }));
            }
        }
        Standing::Departed(None)
    }

    /// The channel of `community_id` whose tag under `generation` is
    /// `tag` ([`stream_of_tag`] over every channel of the merged state).
    pub(crate) fn channel_of_tag(
        &self,
        community_id: &str,
        generation: u64,
        tag: &[u8; 16],
    ) -> Option<String> {
        let state = self.governance_state(community_id)?;
        let keys = self.keys();
        let channels: Vec<(String, rekindle_types::channel_keys::KeyScope)> =
            (state.channels.keys())
                .map(|id| (id.to_hex(), keys.scope_for_text(community_id, *id)))
                .collect();
        let streams = channels.iter().map(|(id, scope)| (id.as_str(), *scope));
        stream_of_tag(&*keys, community_id, streams, generation, tag).map(str::to_owned)
    }

    /// Whether we hold no community-scope key of `community_id` yet (a
    /// joiner before its first key arrives).
    fn awaits_first_key(&self, community_id: &str) -> bool {
        self.keys()
            .current_epoch(
                community_id,
                rekindle_types::channel_keys::KeyScope::Community,
            )
            .is_none()
    }

    /// Apply one chain entry of `author`'s outbox: verified, linked to its
    /// channel by tag, its post ingested. `Err` only when the store failed
    /// (the cursor stays and the next trigger reads it again).
    async fn apply_outbox_entry(
        &self,
        read: &OutboxRead<'_>,
        entry: &ChainEntry,
    ) -> Result<(), rekindle_channel::ChannelError> {
        let (community_id, author) = (read.community_id, read.author);
        let at = ChainPosition {
            anchor: community_id,
            chain_seq: entry.seq,
        };
        let decoded = match decode_outbox_entry(&entry.ciphertext, at, author) {
            Ok(decoded) => decoded,
            Err(error) => {
                tracing::warn!(community_id, author = %hex::encode(author), seq = entry.seq, %error, "channel outbox entry did not verify");
                return Ok(());
            }
        };
        self.check_link(read, entry.seq, &decoded);
        let Some(channel_id) =
            self.channel_of_tag(community_id, decoded.generation, &decoded.channel_tag)
        else {
            // A joiner holds no community key until its first `MekTransfer`
            // (D11, D16): no tag maps yet, so the cursor stays on this entry
            // and the catch-up the key's install starts reads it again.
            // Once a key is held, an entry no key maps is of a channel we do
            // not read or from before our keys, and the chain covers it.
            if self.awaits_first_key(community_id) {
                return Err(rekindle_channel::ChannelError::MekMissing {
                    community: community_id.to_owned(),
                    channel: String::new(),
                });
            }
            tracing::debug!(
                community_id,
                seq = entry.seq,
                "channel outbox entry of no channel we read"
            );
            return Ok(());
        };
        let author_hex = hex::encode(author);
        let (message, forwarded_from_author) = match decoded.entry {
            ChannelRecordEntry::Message(m) => (m, None),
            ChannelRecordEntry::Forward(f) => (
                ChannelMessage {
                    sequence: f.sequence,
                    sender_pseudonym: f.sender_pseudonym,
                    ciphertext: f.content_snapshot,
                    mek_generation: f.mek_generation,
                    timestamp: f.timestamp,
                    reply_to: None,
                    lamport_ts: f.lamport_ts,
                    message_id: f.message_id,
                    attachment: None,
                    flags: 0,
                    mentioned_pseudonyms: Vec::new(),
                    mentioned_roles: Vec::new(),
                },
                Some(f.original_author),
            ),
            change @ (ChannelRecordEntry::Edit(_) | ChannelRecordEntry::Delete(_)) => {
                self.apply_outbox_change(read, entry, &channel_id, decoded.stream_seq, &change)
                    .await;
                return Ok(());
            }
            // A holder's possession of a file: the files lookup reads it
            // (`read_author_outboxes`), nothing is applied.
            ChannelRecordEntry::AttachmentCached(_) => return Ok(()),
            other => {
                tracing::debug!(community_id, seq = entry.seq, kind = ?std::mem::discriminant(&other), "channel outbox entry kind not read yet");
                return Ok(());
            }
        };
        // An entry speaks only for the author whose chain holds it.
        if message.sender_pseudonym != author_hex {
            tracing::warn!(
                community_id,
                seq = entry.seq,
                "channel outbox entry names another author"
            );
            return Ok(());
        }
        let post = ReceivedPost {
            community_id: community_id.to_owned(),
            channel_id: channel_id.clone(),
            at: ReceivedAt::Outbox {
                stream_seq: decoded.stream_seq,
                departure_head: read.departure_head.clone(),
            },
            sequence: decoded.stream_seq,
            message,
            forwarded_from_author,
        };
        match self.ingest_as(post, read.backfill).await? {
            Ingested::Dropped(reason) => {
                tracing::debug!(community_id, channel_id, ?reason, "outbox post dropped");
            }
            Ingested::Undecryptable => {
                tracing::debug!(community_id, channel_id, "outbox post no held key opens");
            }
            _ => {}
        }
        Ok(())
    }

    /// A broken `prev` link is recorded as an anomaly (plan E3.3
    /// "Per-author chain"): logged and written to the audit chain; the
    /// entry is still applied, as its signature at its seq holds.
    fn check_link(
        &self,
        read: &OutboxRead<'_>,
        seq: u64,
        decoded: &rekindle_codec::member_record::OutboxEntry,
    ) {
        let Before::Known(before) = read.before else {
            return;
        };
        if links_to(decoded, before) {
            return;
        }
        let author = hex::encode(read.author);
        tracing::warn!(community_id = read.community_id, %author, member_record = read.member_record, seq, "channel outbox chain broken: the entry does not link to the one before it");
        let (Ok(owner), Ok(db)) = (self.ctx().require_owner(), self.ctx().db.current()) else {
            return;
        };
        crate::host::audit::append(
            &self.ctx().audit_chain,
            &db,
            &self.ctx().vault,
            &owner,
            rekindle_audit::AuditKind::ChainBroken,
            serde_json::json!({
                "community_id": read.community_id,
                "author": author,
                "member_record": read.member_record,
                "seq": seq,
            }),
            self.ctx().clock.now_ms_i64(),
        );
    }

    /// Apply an edit or delete read at chain seq `entry.seq` of the
    /// author's outbox to the post it names (rules E1–E3, the edit's body
    /// through D26). A departed author's change is kept only at or below
    /// its departure's head, as its posts are.
    async fn apply_outbox_change(
        &self,
        read: &OutboxRead<'_>,
        entry: &ChainEntry,
        channel_id: &str,
        stream_seq: u64,
        change: &ChannelRecordEntry,
    ) {
        if read.departed && read.departure_head.is_none() {
            tracing::debug!(
                community_id = read.community_id,
                seq = entry.seq,
                "a departed author's change past its departure's head is refused"
            );
            return;
        }
        let author = PseudonymKey(*read.author);
        let source = super::changes::ChangeSource {
            community_id: read.community_id,
            channel_id,
            record_key: read.member_record,
            subkey_index: OUTBOX,
            outbox_seq: Some(stream_seq),
            author: &author,
        };
        self.apply_changes(source, std::slice::from_ref(change))
            .await;
    }
}
