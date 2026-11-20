//! The per-writer stream engine (plan E2.5, D23): Telegram's update engine
//! with each writer's own record as the box. A stream has one writer, whose
//! append-only chain (`rekindle_records::chain`) is authoritative, and a
//! cursor here: the highest of the writer's sequence numbers we passed.
//!
//! - **Receive rule** ([`classify`]): apply when `cursor + 1 == seq`, pass a
//!   lower one as a duplicate, hold any other (a gap) for the diff.
//! - **Diff** ([`StreamEngine::diff`]): read the chain after the cursor and
//!   apply each entry in order; one diff runs per stream at a time, and a
//!   trigger that lands during one makes it read again before it ends.
//! - **Cursor**: the [`StreamSink`] commits it only after the entry's effect
//!   commits, so a crash between the two re-applies an entry the sink
//!   recognises as a duplicate: nothing is lost or doubled.
//! - **Held prefix** (live finding L-ACK1): an entry the sink could not
//!   hold yet ([`Held::Pending`]: its content not received, decrypted and
//!   stored) stops the pass with the cursor below it, so the cursor, which
//!   a receipt reports (`DmAck.through`, D22), is always the contiguous
//!   prefix of entries held; a head read past it never moves it.
//! - **Gap**: when the writer's retire mark overtook the cursor, the sink
//!   stores and shows a `HistoryGap` and the cursor moves past it. Loss is
//!   never silent.
//! - **Caught up**: each diff that drains its stream reports `CaughtUp`.
//!
//! It is Veilid-free: the chain is read through `&dyn RecordPort`.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use parking_lot::Mutex;

use rekindle_records::chain::{self, ChainEntry, ChainError};
use rekindle_records::port::RecordPort;

/// The stream of a writer's direct messages to us.
pub const DM_STREAM: &str = "dm";

/// What the receive rule does with entry `seq` at `cursor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Apply,
    Duplicate,
    /// A gap before it: held until the diff fills it.
    Hold,
}

/// The receive rule.
#[must_use]
pub fn classify(cursor: u64, seq: u64) -> Rule {
    match seq.checked_sub(cursor) {
        Some(1) => Rule::Apply,
        Some(0) | None => Rule::Duplicate,
        Some(_) => Rule::Hold,
    }
}

/// The sink failed; the diff stops with the cursor where it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SinkError(pub String);

/// A diff failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Sink(#[from] SinkError),
    /// The chain skipped a sequence number after the cursor.
    #[error("the chain holds {seq} after cursor {cursor}")]
    Hole { cursor: u64, seq: u64 },
}

/// What [`StreamSink::apply`] did with an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// Its effect committed (or it was judged: a duplicate, a refusal, an
    /// entry that can never open), and the cursor with it.
    Stored,
    /// Its content is not held yet (its first copy is still being applied,
    /// our storage failed, or it waits on a friendship's accept): the
    /// cursor stays below it and the pass stops; the next trigger reads it
    /// again.
    Pending,
}

/// Where a stream's entries go: the host's apply and cursor store.
#[async_trait::async_trait]
pub trait StreamSink: Send + Sync {
    /// The cursor of `writer` on `stream` (0: nothing passed).
    async fn cursor(&self, stream: &str, writer: &str) -> Result<u64, SinkError>;
    /// Apply `entry` (or recognise it as one already applied), then commit
    /// the cursor to its seq ([`Held::Stored`]); or leave the cursor below
    /// an entry whose content it does not hold yet ([`Held::Pending`]).
    async fn apply(
        &self,
        stream: &str,
        writer: &str,
        entry: &ChainEntry,
    ) -> Result<Held, SinkError>;
    /// The entries up to `through` are retired unread: store and show the
    /// gap, then commit the cursor to `through`.
    async fn history_gap(&self, stream: &str, writer: &str, through: u64) -> Result<(), SinkError>;
    /// The stream drained.
    async fn caught_up(&self, stream: &str, writer: &str);
}

/// What one [`StreamEngine::diff`] call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Diffed {
    /// Entries applied.
    pub applied: usize,
    /// The gap reported, if the retire mark overtook the cursor.
    pub gap_through: Option<u64>,
    /// Another diff of the stream was running: it reads again instead.
    pub coalesced: bool,
    /// A head the reader knows exists that no read reached
    /// ([`chain::read_log_reaching`]): the cursor stays below it and the
    /// diff does not report caught up.
    pub unreached: Option<u64>,
    /// The entry the sink could not hold yet ([`Held::Pending`]): the
    /// cursor stays below it and the diff does not report caught up.
    pub pending: Option<u64>,
}

/// Which chain a diff reads.
#[derive(Debug, Clone, Copy)]
enum Chain {
    /// A participant record's retiring chain (`HEAD` at its subkey 0).
    Participant,
    /// A never-retired log whose head is `head_subkey`, known to reach
    /// `at_least` (0: no head known).
    Log { head_subkey: u32, at_least: u64 },
}

/// The diffs in flight, one per stream: whether a trigger landed during it.
#[derive(Debug, Default)]
pub struct StreamEngine {
    running: Mutex<HashMap<(String, String), bool>>,
}

impl Chain {
    /// The head a log is known to reach (0: none known).
    fn at_least(self) -> u64 {
        match self {
            Self::Participant => 0,
            Self::Log { at_least, .. } => at_least,
        }
    }
}

impl StreamEngine {
    /// Claim the stream's diff, or mark the running one to read again.
    fn claim(&self, key: &(String, String)) -> bool {
        let mut running = self.running.lock();
        if let Some(again) = running.get_mut(key) {
            *again = true;
            return false;
        }
        running.insert(key.clone(), false);
        true
    }

    /// Release the stream's diff unless a trigger landed during it (then
    /// keep it and read again).
    fn release(&self, key: &(String, String), done: bool) -> bool {
        let mut running = self.running.lock();
        let again = running.get(key).copied().unwrap_or(false);
        if done || !again {
            running.remove(key);
            return true;
        }
        running.insert(key.clone(), false);
        false
    }

    /// Diff `writer`'s stream from `record` (their participant record):
    /// every entry after our cursor, in order, then `CaughtUp`.
    ///
    /// # Errors
    /// The chain read or the sink failed; the cursor stays at the last
    /// committed entry and the next trigger resumes from it.
    pub async fn diff(
        &self,
        port: &dyn RecordPort,
        sink: &dyn StreamSink,
        stream: &str,
        writer: &str,
        record: &str,
    ) -> Result<Diffed, StreamError> {
        self.diff_chain(port, sink, (stream, writer), record, Chain::Participant)
            .await
    }

    /// Diff `writer`'s never-retired log chain whose head is
    /// `record`/`head_subkey` (a member's channel outbox, `M`/4, plan
    /// E3.3), as [`diff`](Self::diff) does a participant record. `at_least`
    /// is a head the reader knows the log reaches (a departure's, 0: none):
    /// a read short of it runs bounded gap reads, and if none reaches it the
    /// diff reports it [`unreached`](Diffed::unreached).
    ///
    /// # Errors
    /// As [`diff`](Self::diff).
    pub async fn diff_log(
        &self,
        port: &dyn RecordPort,
        sink: &dyn StreamSink,
        (stream, writer): (&str, &str),
        record: &str,
        (head_subkey, at_least): (u32, u64),
    ) -> Result<Diffed, StreamError> {
        self.diff_chain(
            port,
            sink,
            (stream, writer),
            record,
            Chain::Log {
                head_subkey,
                at_least,
            },
        )
        .await
    }

    async fn diff_chain(
        &self,
        port: &dyn RecordPort,
        sink: &dyn StreamSink,
        (stream, writer): (&str, &str),
        record: &str,
        chain: Chain,
    ) -> Result<Diffed, StreamError> {
        let key = (stream.to_string(), writer.to_string());
        if !self.claim(&key) {
            return Ok(Diffed {
                coalesced: true,
                ..Diffed::default()
            });
        }
        let mut total = Diffed::default();
        loop {
            let pass = Self::pass(port, sink, (stream, writer), record, chain).await;
            let failed = pass.is_err();
            match pass {
                Ok(d) => {
                    total.applied += d.applied;
                    total.gap_through = d.gap_through.or(total.gap_through);
                    total.unreached = d.unreached;
                    total.pending = d.pending;
                }
                Err(e) => {
                    self.release(&key, true);
                    return Err(e);
                }
            }
            if self.release(&key, failed) {
                break;
            }
        }
        if total.unreached.is_none() && total.pending.is_none() {
            sink.caught_up(stream, writer).await;
        }
        Ok(total)
    }

    async fn pass(
        port: &dyn RecordPort,
        sink: &dyn StreamSink,
        (stream, writer): (&str, &str),
        record: &str,
        source: Chain,
    ) -> Result<Diffed, StreamError> {
        let mut cursor = sink.cursor(stream, writer).await?;
        let read = match source {
            Chain::Participant => chain::read_after(port, record, cursor).await?,
            Chain::Log {
                head_subkey,
                at_least,
            } => chain::read_log_reaching(port, record, head_subkey, cursor, at_least).await?,
        };
        let unreached = (read.last_seq < source.at_least()).then(|| source.at_least());
        let mut gap_through = None;
        if let Some(through) = read.retired_gap {
            sink.history_gap(stream, writer, through).await?;
            cursor = through;
            gap_through = Some(through);
        }
        let mut done = Self::apply_entries(sink, (stream, writer), cursor, &read.entries).await?;
        done.gap_through = gap_through;
        done.unreached = unreached;
        Ok(done)
    }

    /// Apply `entries` (read after `cursor`) in order through the receive
    /// rule, stopping at the first the sink could not hold
    /// ([`Held::Pending`]): the cursor never passes an entry not held.
    async fn apply_entries(
        sink: &dyn StreamSink,
        (stream, writer): (&str, &str),
        mut cursor: u64,
        entries: &[ChainEntry],
    ) -> Result<Diffed, StreamError> {
        let mut done = Diffed::default();
        for entry in entries {
            match classify(cursor, entry.seq) {
                Rule::Duplicate => {}
                Rule::Apply => {
                    if sink.apply(stream, writer, entry).await? == Held::Pending {
                        done.pending = Some(entry.seq);
                        break;
                    }
                    cursor = entry.seq;
                    done.applied += 1;
                }
                Rule::Hold => {
                    return Err(StreamError::Hole {
                        cursor,
                        seq: entry.seq,
                    })
                }
            }
        }
        Ok(done)
    }
}
