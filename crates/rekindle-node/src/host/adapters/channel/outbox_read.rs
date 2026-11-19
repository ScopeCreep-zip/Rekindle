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
//! the chain covers ciphertext.
//!
//! Triggers: the author's or a relayer's `HeadAdvanced` for subkey 4 and a
//! ring change of subkey 4 diff that author; a catch-up (a community's
//! load) diffs every member. Older history stays on the channel records
//! until E3.3 slice 8.

use async_trait::async_trait;
use rekindle_channel::outbox_seal::stream_of_tag;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_codec::member_record::{decode_outbox_entry, ChainPosition};
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::{SinkError, StreamSink};
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::id::PseudonymKey;

use super::ingest::{Ingested, ReceivedAt, ReceivedPost};
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
    /// Read by a catch-up rather than a live trigger.
    backfill: bool,
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
        self.channel
            .apply_outbox_entry(self.community_id, &self.author, entry, self.backfill)
            .await
            .map_err(sink_error)?;
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
        let sink = OutboxSink {
            channel: self,
            community_id,
            author: author.0,
            backfill,
        };
        let (stream, writer) = (outbox_stream(community_id), hex::encode(author.0));
        match engine
            .diff_log(&*port, &sink, (&stream, &writer), member_record, OUTBOX)
            .await
        {
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
    /// the community; plan E3.3 "Cursor and authors").
    pub(crate) async fn catch_up_outboxes(&self, community_id: &str) {
        let Some(state) = self.governance_state(community_id) else {
            return;
        };
        for (author, member) in &state.members {
            self.read_author_outbox(community_id, author, &member.member_record, true)
                .await;
        }
    }

    /// Apply one chain entry of `author`'s outbox: verified, linked to its
    /// channel by tag, its post ingested. `Err` only when the store failed
    /// (the cursor stays and the next trigger reads it again).
    async fn apply_outbox_entry(
        &self,
        community_id: &str,
        author: &[u8; 32],
        entry: &ChainEntry,
        backfill: bool,
    ) -> Result<(), rekindle_channel::ChannelError> {
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
        let Some(state) = self.governance_state(community_id) else {
            return Ok(());
        };
        let keys = self.keys();
        let channels: Vec<(String, rekindle_types::channel_keys::KeyScope)> =
            (state.channels.keys())
                .map(|id| (id.to_hex(), keys.scope_for_text(community_id, *id)))
                .collect();
        let streams = channels.iter().map(|(id, scope)| (id.as_str(), *scope));
        let Some(channel_id) = stream_of_tag(
            &*keys,
            community_id,
            streams,
            decoded.generation,
            &decoded.channel_tag,
        ) else {
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
            channel_id: channel_id.to_owned(),
            at: ReceivedAt::Outbox {
                stream_seq: decoded.stream_seq,
            },
            sequence: decoded.stream_seq,
            message,
            forwarded_from_author,
        };
        match self.ingest_as(post, backfill).await? {
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
}
