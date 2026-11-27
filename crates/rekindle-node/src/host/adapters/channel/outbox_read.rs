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
//! first community key: no entry's tag maps yet, so the entry is held
//! ([`Held::Pending`], not an error) and the cursor stays; the key's
//! install catches every outbox up again, and when that catch-up lands
//! while this read still runs, the engine reads again instead of
//! dropping it (finding CB1: a failed pass drops a trigger that landed
//! during it).
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
//! still falls short leaves the cursor below the head, is not caught up,
//! and stores a history-gap marker (`outbox_gap.rs`).
//!
//! Triggers: the author's or a relayer's `HeadAdvanced` for subkey 4 and a
//! ring change of subkey 4 diff that author; a catch-up (a community's
//! load: an unlock, a join, a community key installed) diffs every member. A
//! thread reply's tag names a thread, not a channel, so it maps to no
//! channel here and is read by the thread view (`read_author_outboxes`).

use rekindle_channel::ingest::GateReason;
use rekindle_channel::outbox_seal::stream_of_tag;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_codec::community::channel_record::{
    ChannelMessage, ChannelPollClose, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
};
use rekindle_codec::member_record::{decode_outbox_entry, links_to, ChainPosition};
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::{Diffed, Held};
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
    ) -> usize {
        if self.ctx().communities.own_removal(community_id).is_some() {
            return 0;
        }
        let engine = {
            let map = self.ctx().communities.map().read();
            map.get(community_id).map(|c| c.outbox_streams.clone())
        };
        let (Some(engine), Ok(port)) = (engine, self.records()) else {
            return 0;
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
        let diffed = engine
            .diff_log(
                &*port,
                &sink,
                (&stream, &writer),
                member_record,
                (OUTBOX, known_head),
            )
            .await;
        // An entry waiting for its key leaves the read short of the head.
        let waiting = diffed.as_ref().is_ok_and(|d| d.pending.is_some());
        let applied = diffed.as_ref().map_or(0, |d| d.applied);
        match diffed {
            Ok(Diffed {
                unreached: Some(head),
                applied,
                ..
            }) => {
                tracing::warn!(community_id, author = %writer, head, applied, "channel outbox head a departure recorded is on no replica we reached: the history keeps the gap");
                self.mark_outbox_gap(community_id, author, head).await;
                return applied;
            }
            Ok(diffed) if diffed.applied > 0 => {
                tracing::info!(community_id, author = %writer, applied = diffed.applied, "channel outbox read");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(community_id, author = %writer, %error, "channel outbox not read yet");
                return 0;
            }
        }
        if waiting {
            tracing::debug!(community_id, author = %writer, "channel outbox entry waits for its key");
            return applied;
        }
        if known_head > 0 {
            self.close_outbox_gap(community_id, author, known_head)
                .await;
        }
        applied
    }

    /// Catch up every member's channel outbox in `community_id` (a load of
    /// the community; plan E3.3 "Cursor and authors"), and each departed
    /// author's whose departures recorded a head of it, so a member joining
    /// later keeps what the author posted before it departed. Returns the
    /// authors read and the entries applied.
    pub(crate) async fn catch_up_outboxes(&self, community_id: &str) -> (usize, usize) {
        let Some(state) = self.governance_state(community_id) else {
            return (0, 0);
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
        let mut applied = 0;
        for (author, record) in &authors {
            applied += (self.read_author_outbox(community_id, author, record, true)).await;
        }
        (authors.len(), applied)
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

    /// The last chain position a departed author's history can include.
    pub(super) async fn departed_lookup_through(
        &self,
        port: &dyn rekindle_records::port::RecordPort,
        community: &str,
        author: &PseudonymKey,
        record: &str,
    ) -> Option<u64> {
        match self.standing(port, community, author, record).await {
            Standing::Departed(Some(head)) => Some(u64::from(head.seq)),
            Standing::Member | Standing::Departed(None) => None,
        }
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

    /// Apply one chain entry of `author`'s outbox: verified, linked to its
    /// channel by tag, its post ingested. [`Held::Pending`] when the entry
    /// waits for its key; `Err` only when the store failed (the cursor
    /// stays and the next trigger reads it again).
    async fn apply_outbox_entry(
        &self,
        read: &OutboxRead<'_>,
        entry: &ChainEntry,
    ) -> Result<Held, rekindle_channel::ChannelError> {
        let (community_id, author) = (read.community_id, read.author);
        let at = ChainPosition {
            anchor: community_id,
            chain_seq: entry.seq,
        };
        let decoded = match decode_outbox_entry(&entry.ciphertext, at, author) {
            Ok(decoded) => decoded,
            Err(error) => {
                tracing::warn!(community_id, author = %hex::encode(author), seq = entry.seq, %error, "channel outbox entry did not verify");
                return Ok(Held::Stored);
            }
        };
        self.check_link(read, entry.seq, &decoded);
        let tag = (decoded.generation, &decoded.channel_tag);
        let stream = || self.post_stream_of_tag(community_id, tag.0, tag.1);
        let (channel_id, parent) = match stream() {
            Some(stream) => stream,
            // A joiner before its first `MekTransfer` (D11, D16), or a
            // reader owed a converged generation's key it does not hold
            // yet: the cursor stays on this entry and the catch-up the key's
            // install starts reads it again (`outbox_hold.rs`).
            None if self.entry_waits_for_key(community_id, tag.0) => return Ok(Held::Pending),
            // Otherwise the entry is of a channel we do not read, from
            // before our keys, or of a stream whose creation
            // has not merged here yet: it passes, and its writer is diffed
            // again when a merge creates a channel (`outbox_unmapped.rs`).
            // Noted before the second look, so a creation merging between
            // the two is either seen here or re-reads the writer.
            None => {
                self.note_unmapped(community_id, author, read.member_record);
                let Some(stream) = stream() else {
                    tracing::info!(
                        community_id,
                        seq = entry.seq,
                        "channel outbox entry of no channel merged here: re-read when one is created"
                    );
                    return Ok(Held::Stored);
                };
                stream
            }
        };
        if parent.is_some() && !matches!(&decoded.entry, ChannelRecordEntry::Message(_)) {
            return Ok(Held::Stored);
        }
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
                    revives: None,
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
                return Ok(Held::Stored);
            }
            // A holder's possession of a file, a reaction, a poll op or a
            // hand raise: readers aggregate them from the authors' outboxes
            // when they are shown (`read_author_outboxes`; rules R1-R3, no
            // shared aggregate), so nothing is applied here.
            // A poll op changes what the poll's readers show: told.
            // A create names its creator's nonce: the poll is derived from
            // it and this outbox's author (the creator, D23).
            ChannelRecordEntry::PollCreate(ChannelPollCreate { nonce, .. }) => {
                let poll_id = rekindle_channel::poll_id_of(&PseudonymKey(*author), &nonce);
                self.poll_changed(community_id, &channel_id, &poll_id);
                return Ok(Held::Stored);
            }
            ChannelRecordEntry::PollVote(ChannelPollVote { poll_id, .. })
            | ChannelRecordEntry::PollClose(ChannelPollClose { poll_id, .. }) => {
                self.poll_changed(community_id, &channel_id, &poll_id);
                return Ok(Held::Stored);
            }
            ChannelRecordEntry::AttachmentCached(_)
            | ChannelRecordEntry::Reaction(_)
            | ChannelRecordEntry::HandRaise(_) => return Ok(Held::Stored),
        };
        // An entry speaks only for the author whose chain holds it.
        if message.sender_pseudonym != author_hex {
            tracing::warn!(
                community_id,
                seq = entry.seq,
                "channel outbox entry names another author"
            );
            return Ok(Held::Stored);
        }
        let generation = message.mek_generation;
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
        if self
            .require_channel_permission(
                community_id,
                Some(parent.as_deref().unwrap_or(&channel_id)),
                rekindle_types::permissions::VIEW_CHANNELS,
            )
            .is_err()
        {
            return Ok(Held::Stored);
        }
        let status = self
            .record_post_admission(
                &post,
                &PseudonymKey(*author),
                &decoded.governance_cut,
                read.member_record,
                parent.is_some(),
            )
            .await?;
        if let Some(held) = status {
            return Ok(held);
        }
        if parent.is_some() {
            self.set_post_admission(&post, read.member_record, None, true)
                .await?;
            return Ok(Held::Stored);
        }
        let ingested = self.ingest_authorized(post.clone(), read.backfill).await?;
        self.set_post_admission(&post, read.member_record, None, false)
            .await?;
        match ingested {
            // The gate judged the author departed while this read took it
            // as a member: its departure merged after the standing was
            // taken, so the entry was judged without the departure head
            // that may keep it. Held, not dropped: the cursor stays on it
            // and it is read again under the departed standing
            // (`OutboxSink::apply`), never passed as from a non-member
            // (ADR 0011: a leaver's writes before its leave stay valid).
            Ingested::Dropped(GateReason::NotMember) if !read.departed => {
                tracing::debug!(
                    community_id,
                    channel_id,
                    "outbox post judged under a standing its departure overtook: read again"
                );
                return Ok(Held::Pending);
            }
            Ingested::Dropped(reason) => {
                tracing::debug!(community_id, channel_id, ?reason, "outbox post dropped");
            }
            // The body names a generation we are owed and do not hold yet
            // (behind, or before our first `MekTransfer`): held, so the
            // catch-up the key's install starts reads it again and it
            // arrives with its text (`outbox_hold.rs`). One whose key we
            // hold is damaged: stored visibly unreadable, never skipped
            // past invisibly. Neither: history from before our keys.
            Ingested::Undecryptable if self.entry_waits_for_key(community_id, generation) => {
                return Ok(Held::Pending);
            }
            Ingested::Undecryptable => {
                if !self.store_undecryptable(post).await? {
                    tracing::debug!(
                        community_id,
                        channel_id,
                        "outbox post of a key from before ours"
                    );
                }
            }
            // `Withheld`: we may not view its channel; the cursor passes it
            // and the merge that opens the channel re-reads every outbox
            // (`governance/view_gain.rs`).
            _ => {}
        }
        Ok(Held::Stored)
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
            stream_seq,
            author: &author,
        };
        self.apply_changes(source, std::slice::from_ref(change))
            .await;
    }
}
mod sink;
use sink::{OutboxSink, Standing};

#[cfg(test)]
mod tests;
