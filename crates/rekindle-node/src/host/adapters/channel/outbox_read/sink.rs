//! The outbox sink: one author's channel outbox as the D23 engine applies it
//! (split from `outbox_read.rs` at its 600-line ceiling).

use async_trait::async_trait;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_codec::member_record::entry_hash;
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::{Held, SinkError, StreamSink};
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::id::PseudonymKey;

use super::super::received::OutboxHead;
use super::super::HostChannel;
use super::{Before, OutboxRead};

fn sink_error(e: impl std::fmt::Display) -> SinkError {
    SinkError(e.to_string())
}

/// One author's outbox as the D23 engine applies it.
pub(super) struct OutboxSink<'a> {
    pub(super) channel: &'a HostChannel,
    pub(super) community_id: &'a str,
    pub(super) author: [u8; 32],
    /// The author's member record.
    pub(super) member_record: &'a str,
    /// Read by a catch-up rather than a live trigger.
    pub(super) backfill: bool,
    /// Whether the author is a member, or which departure head keeps its
    /// entries.
    pub(super) standing: Standing,
    /// The chain the entries are read from (the link check's reads).
    pub(super) port: &'a dyn rekindle_records::port::RecordPort,
    /// The seq and stored-bytes hash of the entry this diff applied last.
    pub(super) last: parking_lot::Mutex<Option<(u64, [u8; 32])>>,
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

    async fn apply(
        &self,
        stream: &str,
        writer: &str,
        entry: &ChainEntry,
    ) -> Result<Held, SinkError> {
        let standing = self.current_standing().await;
        let departure_head = match &standing {
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
            departed: matches!(standing, Standing::Departed(_)),
            departure_head,
            before: self.before(entry.seq).await,
        };
        let held = self
            .channel
            .apply_outbox_entry(&read, entry)
            .await
            .map_err(sink_error)?;
        if held == Held::Pending {
            // Waiting for its key: the cursor stays on it, and a trigger
            // that landed meanwhile (the key's install) reads again (CB1).
            return Ok(Held::Pending);
        }
        *self.last.lock() = Some((entry.seq, entry_hash(&entry.ciphertext)));
        self.advance(stream, writer, entry.seq)
            .await
            .map(|()| Held::Stored)
    }

    /// An outbox is never retired, so its read reports no gap (nor an
    /// entry after one to judge it by).
    async fn history_gap(
        &self,
        stream: &str,
        writer: &str,
        through: u64,
        _next: Option<&ChainEntry>,
    ) -> Result<(), SinkError> {
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

impl OutboxSink<'_> {
    /// The author's standing for the entry about to be applied. The read
    /// took it when it started; a departure merged since (the leave of an
    /// author whose withheld last write this read applies) makes the ingest
    /// gate judge the author departed, so a member standing is taken again
    /// and the entry is kept at or below the departure's head the chain
    /// reproduces, as every later read keeps it, never dropped as from a
    /// non-member with the cursor moved past it (ADR 0011: a leaver's
    /// writes before its leave stay valid).
    async fn current_standing(&self) -> Standing {
        let author = PseudonymKey(self.author);
        let stale = matches!(self.standing, Standing::Member)
            && (self.channel.governance_state(self.community_id))
                .is_some_and(|state| state.is_departed(&author));
        if !stale {
            return self.standing.clone();
        }
        (self.channel)
            .standing(self.port, self.community_id, &author, self.member_record)
            .await
    }
}

/// An outbox author's standing in the merged state.
#[derive(Clone)]
pub(super) enum Standing {
    Member,
    /// Departed: the highest of its departure heads of this outbox that
    /// the chain reproduces (`None`: none does, so none of its entries is
    /// kept).
    Departed(Option<OutboxHead>),
}
