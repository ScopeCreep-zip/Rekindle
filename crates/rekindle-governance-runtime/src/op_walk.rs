//! The joiner's walk of the op DAG (plan E3.2 S7c.1, walk cost).
//!
//! [`walk`] reads the op set from a member's `Frontier` by `OpRef`, one
//! DAG level per pass. A level's fetches run concurrently, at most
//! [`WALK_FETCHES_IN_FLIGHT`] at once (a fixed cap, as Synapse's background
//! device-list resync bounds its fan-out), so a 64-head level of shallow
//! joins does not open 64 forced `HEAD` reads at once and does not wait
//! for them one by one either. Every fetch of one walk shares a
//! [`WalkLogCache`]: an author's op log is read once (from its start, on
//! the first fetch for that author) and every later op of that author is
//! served from the cache, where a fetch per op re-read the log each time.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::sync::Arc;

use futures::stream::{self, StreamExt, TryStreamExt};
use rekindle_records::port::RecordPort;
use rekindle_types::dht_layout::member_record::GOV_HEAD;
use rekindle_types::governance::{OpHash, OpRef};

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::op_log::{fetch_held, HeldOp};
use crate::verify_cache::{verify_held_cached, OpVerifyCache};

/// How many `fetch_held` calls one walk level keeps in flight.
pub const WALK_FETCHES_IN_FLIGHT: usize = 16;

/// What one read of an author's op log returned: its entries after the
/// cursor, by chain seq, and the chain's last seq at the read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LogRead {
    pub entries: Vec<(u64, Vec<u8>)>,
    pub last_seq: u64,
}

/// One author's op log as this walk has read it.
#[derive(Debug, Default)]
struct AuthorLog {
    entries: BTreeMap<u64, Vec<u8>>,
    /// The chain's last seq at the newest read; `None` before any read.
    read_to: Option<u64>,
}

/// The op logs one walk has read, by author member record. Created per
/// [`walk`] (and per rebuild or sweep) and passed down to
/// `GovernanceRuntimeDeps::fetch_governance_op`; dropped with it, so a
/// later walk reads the logs afresh.
#[derive(Debug, Default)]
pub struct WalkLogCache {
    logs: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<AuthorLog>>>>,
}

impl WalkLogCache {
    /// The entry at chain seq `seq` of `author_record`'s op log: from the
    /// cache when this walk has read it; else `read_after(cursor)` reads
    /// the entries after `cursor` (0 on the author's first fetch, the last
    /// read's last seq after that) and they are kept. `None`: a read that
    /// covered `seq` did not hold it. Fetches of one author wait on each
    /// other, so concurrent fetches of one log read it once.
    ///
    /// # Errors
    /// `read_after` failed.
    pub async fn entry<F, Fut, E>(
        &self,
        author_record: &str,
        seq: u64,
        read_after: F,
    ) -> Result<Option<Vec<u8>>, E>
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<LogRead, E>>,
    {
        let log = Arc::clone(
            self.logs
                .lock()
                .entry(author_record.to_owned())
                .or_default(),
        );
        let mut log = log.lock().await;
        if let Some(entry) = log.entries.get(&seq) {
            return Ok(Some(entry.clone()));
        }
        if log.read_to.is_some_and(|last| seq <= last) {
            return Ok(None);
        }
        let read = read_after(log.read_to.unwrap_or(0)).await?;
        log.entries.extend(read.entries);
        log.read_to = Some(log.read_to.map_or(read.last_seq, |l| l.max(read.last_seq)));
        Ok(log.entries.get(&seq).cloned())
    }
}

/// The encoded op `op_ref` names from its author's op log (`M`/2, chain
/// seq `author_seq + 1`), through `logs`: the log is read with
/// `rekindle_records::chain::read_log_after` once per walk, from its start,
/// and continued after its last read seq when a later op is asked for.
/// What `GovernanceRuntimeDeps::fetch_governance_op` runs on the Host.
///
/// # Errors
/// A read failed, a page is missing, or a value is malformed.
pub async fn fetch_from_log(
    port: &dyn RecordPort,
    logs: &WalkLogCache,
    op_ref: &OpRef,
) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
    let record = op_ref.author_record.as_str();
    logs.entry(record, op_ref.author_seq + 1, |cursor| async move {
        let read = rekindle_records::chain::read_log_after(port, record, GOV_HEAD, cursor)
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("read op log: {e}")))?;
        Ok(LogRead {
            entries: read
                .entries
                .into_iter()
                .map(|entry| (entry.seq, entry.ciphertext))
                .collect(),
            last_seq: read.last_seq,
        })
    })
    .await
}

/// Walk the op DAG of `anchor` from `start` (a member's `Frontier`) by
/// `OpRef`, through deps, until a pass fetches nothing new: the joiner's
/// read of the op set with nobody online. Nothing is stored; the caller
/// stores the walked ops once its community row exists.
///
/// # Errors
/// A fetch failed.
pub async fn walk<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    start: &[OpRef],
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    walk_seeded(deps, anchor, start, Vec::new()).await
}

/// [`walk`] from `start` with `seed` already held: the ops of our own
/// chain a re-join holds from the community it kept as read-only history
/// after our removal (`join_walk`), which our op log may not hold (an op
/// queued there when we were removed). `seed` is returned with the walked
/// ops, and the deps it misses are walked too.
///
/// # Errors
/// A fetch failed.
pub async fn walk_seeded<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    start: &[OpRef],
    seed: Vec<HeldOp>,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    let logs = WalkLogCache::default();
    let verdicts = deps.op_verify_cache(anchor);
    let logs = &logs;
    walk_from(anchor, start, seed, &verdicts, |op_ref| async move {
        fetch_held(deps, &op_ref, logs).await
    })
    .await
}

/// [`walk`] over `fetch`: each level's untried refs are fetched with at
/// most [`WALK_FETCHES_IN_FLIGHT`] in flight; the next level is what the
/// walked set still misses.
///
/// # Errors
/// A fetch failed.
pub async fn walk_with<F, Fut>(
    anchor: &str,
    start: &[OpRef],
    verdicts: &OpVerifyCache,
    fetch: F,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError>
where
    F: Fn(OpRef) -> Fut,
    Fut: Future<Output = Result<Option<HeldOp>, GovernanceRuntimeError>>,
{
    walk_from(anchor, start, Vec::new(), verdicts, fetch).await
}

/// [`walk_with`] with `seed` held before the first level.
async fn walk_from<F, Fut>(
    anchor: &str,
    start: &[OpRef],
    seed: Vec<HeldOp>,
    verdicts: &OpVerifyCache,
    fetch: F,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError>
where
    F: Fn(OpRef) -> Fut,
    Fut: Future<Output = Result<Option<HeldOp>, GovernanceRuntimeError>>,
{
    let mut have: BTreeSet<OpHash> = seed.iter().map(|op| op.hash).collect();
    let mut tried: BTreeSet<(OpHash, String)> = BTreeSet::new();
    let mut wanted: Vec<OpRef> = start.to_vec();
    if !seed.is_empty() {
        wanted.extend(verify_held_cached(anchor, &seed, verdicts).missing);
    }
    let mut held = seed;
    loop {
        let level: Vec<OpRef> = locators(&wanted, &held)
            .into_iter()
            .filter(|r| {
                !have.contains(&r.op_hash) && tried.insert((r.op_hash, r.author_record.clone()))
            })
            .collect();
        let fetched: Vec<HeldOp> = stream::iter(level.into_iter().map(&fetch))
            .buffer_unordered(WALK_FETCHES_IN_FLIGHT)
            .try_filter_map(|op| async move { Ok(op) })
            .try_collect()
            .await?;
        // Two locators of one op may both answer within a level.
        let fetched: Vec<HeldOp> = fetched
            .into_iter()
            .filter(|op| have.insert(op.hash))
            .collect();
        if fetched.is_empty() {
            return Ok(held);
        }
        held.extend(fetched);
        wanted = verify_held_cached(anchor, &held, verdicts).missing;
    }
}

/// Where the ops `wanted` names may be read: each ref as cited, then the
/// same op at every other member record of its author that `held` shows.
/// A re-joiner's earlier ops are cited at the record of its first join,
/// which may never have held them (an op still queued there when it was
/// removed); the re-join writes its whole chain onto its fresh record at
/// the same chain seq (`mirror_own_chain` on the Host), and later ops cite
/// that record. ADR 0011: an `OpRef` is a locator, "so any op is fetchable
/// without a directory"; the op hash, checked on every fetch
/// (`held_from_encoded`), names the op, and a record only says where to
/// look (as a Matrix server fetches a missing event from any server in the
/// room, `/get_missing_events`, not only from its sender's).
#[must_use]
pub fn locators(wanted: &[OpRef], held: &[HeldOp]) -> Vec<OpRef> {
    let mut records: BTreeMap<[u8; 32], BTreeSet<&str>> = BTreeMap::new();
    for op in held {
        (records.entry(op.author.0).or_default()).insert(op.author_record.as_str());
    }
    let mut out = Vec::with_capacity(wanted.len());
    for r in wanted {
        out.push(r.clone());
        let others = (records.get(&r.author.0).into_iter().flatten())
            .filter(|record| **record != r.author_record);
        out.extend(others.map(|record| OpRef {
            author_record: (*record).to_owned(),
            ..r.clone()
        }));
    }
    out
}

#[cfg(test)]
#[path = "op_walk_tests.rs"]
mod tests;
