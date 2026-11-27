//! The channel outbox chain writer (plan E3.3, `M`/4): a drain's entries
//! are signed at their N2 chain seqs, hash-linked to the author's previous
//! entry (`prev`), and appended to our member record's outbox chain as
//! sealed pages (`rekindle_records::chain::append_exact`, `M`/4 =
//! `dht_layout::member_record::OUTBOX`), never retired.
//!
//! An entry's `streamSeq` is frozen at creation, so a drain that committed
//! and retries finds its entries on the chain's newest page and does not
//! append them again.

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_codec::member_record::outbox_entry::CHANNEL_TAG_LEN;
use rekindle_codec::member_record::{decode_outbox_entry, entry_hash, ChainPosition, OutboxEntry};
use rekindle_records::chain::{self, ChainError};
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::OUTBOX;

use crate::error::ChannelError;

/// One entry a drain commits, before its chain position is known.
#[derive(Debug, Clone)]
pub struct PendingEntry {
    pub governance_cut: Vec<rekindle_types::governance::OpRef>,
    pub channel_tag: [u8; CHANNEL_TAG_LEN],
    /// The per-(author, tag) counter assigned when the entry was queued.
    pub stream_seq: u64,
    pub generation: u64,
    pub entry: ChannelRecordEntry,
}

/// The newest entry of the author's chain: its seq and stored bytes.
#[derive(Debug, Clone)]
pub struct ChainTip {
    pub seq: u64,
    pub bytes: Vec<u8>,
}

/// Who writes and where: our member record, its owner keypair `K_M(p)`,
/// the community anchor and our pseudonym.
pub struct OutboxWriter<'a> {
    pub port: &'a dyn RecordPort,
    pub member_record: &'a str,
    pub owner: &'a str,
    pub anchor: &'a str,
    pub author: &'a SigningKey,
}

/// What an append did with each pending entry, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboxCommit {
    /// Appended now, at this chain seq.
    Appended(u64),
    /// Already on the chain from an earlier attempt, at this chain seq.
    Already(u64),
}

/// A drain's result: the outcomes of a prefix of its entries, and why the
/// rest were not appended.
#[derive(Debug)]
pub struct AppendReport {
    pub committed: Vec<OutboxCommit>,
    pub error: Option<ChannelError>,
}

/// Refuse an entry no sealed page holds alone (plan E3.3: one page's
/// subkey minus the page overhead), as its append would be.
///
/// # Errors
/// [`ChannelError::EntryTooLarge`].
pub fn check_entry_size(bytes: &[u8]) -> Result<(), ChannelError> {
    chain::page_runs(&[bytes.to_vec()])
        .map(|_| ())
        .map_err(|error| match error {
            ChainError::TooLarge { cap, .. } => ChannelError::EntryTooLarge {
                size: bytes.len(),
                max: cap,
            },
            other => ChannelError::Storage(other.to_string()),
        })
}

/// Sign `pending` at the chain seqs after `tip`, each `prev` the hash of
/// the entry before it.
///
/// # Errors
/// An entry does not encode, or is over one page.
pub fn seal_batch(
    tip: Option<&ChainTip>,
    anchor: &str,
    author: &SigningKey,
    committed_at: u64,
    pending: &[PendingEntry],
) -> Result<Vec<Vec<u8>>, ChannelError> {
    let mut seq = tip.map_or(0, |t| t.seq);
    let mut prev = tip.map(|t| entry_hash(&t.bytes));
    let mut out = Vec::with_capacity(pending.len());
    for p in pending {
        seq += 1;
        let bytes = OutboxEntry {
            governance_cut: p.governance_cut.clone(),
            prev,
            channel_tag: p.channel_tag,
            stream_seq: p.stream_seq,
            generation: p.generation,
            committed_at,
            entry: p.entry.clone(),
        }
        .encode(
            ChainPosition {
                anchor,
                chain_seq: seq,
            },
            author,
        )
        .map_err(|e| ChannelError::Encoding(e.to_string()))?;
        check_entry_size(&bytes)?;
        prev = Some(entry_hash(&bytes));
        out.push(bytes);
    }
    Ok(out)
}

impl OutboxWriter<'_> {
    /// Our chain's newest page, decoded: its tip and the `(tag,
    /// streamSeq)` of each entry on it, with its chain seq.
    async fn newest_page(&self) -> Result<(Option<ChainTip>, Vec<(EntryKey, u64)>), ChannelError> {
        let head = chain::head_at(self.port, self.member_record, OUTBOX)
            .await
            .map_err(|e| storage(&e))?;
        let Some(head) = head else {
            return Ok((None, Vec::new()));
        };
        let read = chain::read_log_after(
            self.port,
            self.member_record,
            OUTBOX,
            head.first_seq.saturating_sub(1),
        )
        .await
        .map_err(|e| storage(&e))?;
        let public = self.author.verifying_key().to_bytes();
        let mut keys = Vec::with_capacity(read.entries.len());
        for e in &read.entries {
            let at = ChainPosition {
                anchor: self.anchor,
                chain_seq: e.seq,
            };
            // Our own chain: an entry we cannot verify is not ours to skip.
            let decoded = decode_outbox_entry(&e.ciphertext, at, &public).map_err(|err| {
                ChannelError::Encoding(format!("own outbox entry {}: {err}", e.seq))
            })?;
            keys.push(((decoded.channel_tag, decoded.stream_seq), e.seq));
        }
        let tip = read.entries.last().map(|e| ChainTip {
            seq: e.seq,
            bytes: e.ciphertext.clone(),
        });
        Ok((tip, keys))
    }

    /// Append `pending` (one drain, in queue order) to our outbox chain.
    /// Entries already on the newest page (a retried drain) are reported
    /// [`OutboxCommit::Already`] and not appended again.
    pub async fn append(&self, pending: &[PendingEntry], committed_at: u64) -> AppendReport {
        let (tip, on_page) = match self.newest_page().await {
            Ok(found) => found,
            Err(error) => {
                return AppendReport {
                    committed: Vec::new(),
                    error: Some(error),
                }
            }
        };
        let already = |p: &PendingEntry| {
            on_page
                .iter()
                .find(|(key, _)| *key == (p.channel_tag, p.stream_seq))
                .map(|(_, seq)| *seq)
        };
        let mut committed = Vec::with_capacity(pending.len());
        // A drain commits in queue order, so its committed entries are a
        // prefix.
        for p in pending {
            match already(p) {
                Some(seq) => committed.push(OutboxCommit::Already(seq)),
                None => break,
            }
        }
        let fresh = &pending[committed.len()..];
        let error = self
            .append_fresh(tip.as_ref(), fresh, committed_at, &mut committed)
            .await;
        AppendReport { committed, error }
    }

    async fn append_fresh(
        &self,
        tip: Option<&ChainTip>,
        fresh: &[PendingEntry],
        committed_at: u64,
        committed: &mut Vec<OutboxCommit>,
    ) -> Option<ChannelError> {
        if fresh.is_empty() {
            return None;
        }
        let sealed = match seal_batch(tip, self.anchor, self.author, committed_at, fresh) {
            Ok(sealed) => sealed,
            Err(error) => return Some(error),
        };
        let runs = match chain::page_runs(&sealed) {
            Ok(runs) => runs,
            Err(error) => return Some(storage(&error)),
        };
        let mut next = tip.map_or(0, |t| t.seq) + 1;
        for run in runs {
            if let Err(error) =
                chain::append_exact(self.port, self.member_record, OUTBOX, self.owner, next, run)
                    .await
            {
                return Some(storage(&error));
            }
            for _ in run {
                committed.push(OutboxCommit::Appended(next));
                next += 1;
            }
        }
        None
    }
}

type EntryKey = ([u8; CHANNEL_TAG_LEN], u64);

fn storage(error: &ChainError) -> ChannelError {
    ChannelError::Storage(format!("channel outbox chain: {error}"))
}

#[cfg(test)]
#[path = "outbox_chain_tests.rs"]
mod tests;
