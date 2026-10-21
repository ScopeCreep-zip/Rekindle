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

/// Where a stream's entries go: the host's apply and cursor store.
#[async_trait::async_trait]
pub trait StreamSink: Send + Sync {
    /// The cursor of `writer` on `stream` (0: nothing passed).
    async fn cursor(&self, stream: &str, writer: &str) -> Result<u64, SinkError>;
    /// Apply `entry` (or recognise it as one already applied), then commit
    /// the cursor to its seq.
    async fn apply(&self, stream: &str, writer: &str, entry: &ChainEntry) -> Result<(), SinkError>;
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
}

/// The diffs in flight, one per stream: whether a trigger landed during it.
#[derive(Debug, Default)]
pub struct StreamEngine {
    running: Mutex<HashMap<(String, String), bool>>,
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
        let key = (stream.to_string(), writer.to_string());
        if !self.claim(&key) {
            return Ok(Diffed {
                coalesced: true,
                ..Diffed::default()
            });
        }
        let mut total = Diffed::default();
        loop {
            let pass = Self::pass(port, sink, stream, writer, record).await;
            let failed = pass.is_err();
            match pass {
                Ok(d) => {
                    total.applied += d.applied;
                    total.gap_through = d.gap_through.or(total.gap_through);
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
        sink.caught_up(stream, writer).await;
        Ok(total)
    }

    async fn pass(
        port: &dyn RecordPort,
        sink: &dyn StreamSink,
        stream: &str,
        writer: &str,
        record: &str,
    ) -> Result<Diffed, StreamError> {
        let mut cursor = sink.cursor(stream, writer).await?;
        let read = chain::read_after(port, record, cursor).await?;
        let mut done = Diffed::default();
        if let Some(through) = read.retired_gap {
            sink.history_gap(stream, writer, through).await?;
            cursor = through;
            done.gap_through = Some(through);
        }
        for entry in &read.entries {
            match classify(cursor, entry.seq) {
                Rule::Duplicate => {}
                Rule::Apply => {
                    sink.apply(stream, writer, entry).await?;
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
