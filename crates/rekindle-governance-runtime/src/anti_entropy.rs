//! Governance anti-entropy (plan E3.2 "Watching", `evidence/slot-integrity.md`
//! §4.3 (iii)): every [`SWEEP_PERIOD`] a member inspects [`SWEEP_SAMPLE`]
//! random member records and, on login or reconnect, all of them. Each
//! inspected record's `Frontier` (`M`/3) seq is compared with the one last
//! read; a changed frontier is read and every head we do not hold is fetched
//! by its `OpRef` and stored. The rebuild that follows merges them, which
//! records an equivocating author's ops as `Equivocation` (D15): a forked
//! chain shows up as two ops cited by different frontiers. A frontier whose
//! heads equal ours carries the roster checksum its member merged; one that
//! differs from ours is a divergence signal, warned once and never a
//! rebuild, and our state is rebuilt only when it lags our own merge (D15's
//! convergence check, [`check_convergence`]). Every path that reads a
//! frontier shares one [`SeenFrontiers`] per community. These records are
//! deliberately unwatched by most members (the ring watches 4 of them), so
//! this is the one app inspect D4 allows.

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_governance::checksum::roster_checksum;
use rekindle_records::port::OpenIntent;
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{Frontier, OpHash};

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::op_log;
pub use crate::seen_frontiers::SeenFrontiers;
use crate::seen_frontiers::{op_set_digest, MergedView};

/// Member records inspected per periodic sweep.
pub const SWEEP_SAMPLE: usize = 16;
/// The periodic sweep's period.
pub const SWEEP_PERIOD: Duration = Duration::from_secs(60);

/// Which records a sweep inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// Every admitted member's record and the creator's (login, reconnect).
    Full,
    /// [`SWEEP_SAMPLE`] of them, chosen at random.
    Sample,
}

/// The member records of `community_id`'s merged state, each member's
/// further device records (`M(p,d)`, ADR 0022 rule 3: a second device's
/// ops and frontier live there, the record `op_record` resolves) and the
/// creator's, in record order. A gossiped op is merged only once read at
/// its locator (lead ruling 2026-11-26 item 1), so every op log is swept.
fn member_records<D: GovernanceRuntimeDeps>(deps: &D, community_id: &str) -> Vec<String> {
    let mut records: BTreeSet<String> = deps
        .governance_state(community_id)
        .map(|s| {
            let devices = (s.members.keys())
                .flat_map(|p| crate::watch_ring::device_records(&s, community_id, p));
            (s.members.values())
                .map(|m| m.member_record.clone())
                .chain(devices)
                .collect()
        })
        .unwrap_or_default();
    if let Some(cert) = deps.genesis_cert(community_id) {
        records.insert(cert.creator_member_record);
    }
    records.into_iter().collect()
}

/// `n` of `records`, by a partial Fisher–Yates shuffle over the rng port.
fn sample<D: GovernanceRuntimeDeps>(deps: &D, mut records: Vec<String>, n: usize) -> Vec<String> {
    let rng = deps.rng();
    let take = n.min(records.len());
    for i in 0..take {
        let remaining = (records.len() - i) as u64;
        let j = i + usize::try_from(rng.below(remaining)).unwrap_or(0);
        records.swap(i, j);
    }
    records.truncate(take);
    records
}

/// Sweep `community_id`: returns how many heads were stored, which the
/// caller's `rebuild_governance` merges.
///
/// # Errors
/// No cert is pinned, or the op store failed.
pub async fn sweep<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    kind: Sweep,
    seen: &SeenFrontiers,
) -> Result<usize, GovernanceRuntimeError> {
    if deps.genesis_cert(community_id).is_none() {
        return Err(GovernanceRuntimeError::GenesisCertMissing(
            community_id.to_owned(),
        ));
    }
    let records = member_records(deps, community_id);
    let records = match kind {
        Sweep::Full => records,
        Sweep::Sample => sample(deps, records, SWEEP_SAMPLE),
    };
    let mut known: BTreeSet<OpHash> = deps
        .op_store()
        .governance_ops(community_id)
        .await?
        .iter()
        .map(|op| op.hash)
        .collect();
    let mut stored = 0;
    for record in &records {
        stored += inspect_one(
            deps,
            community_id,
            record,
            seen,
            &mut known,
            FrontierRead::Sweep,
        )
        .await?;
    }
    Ok(stored)
}

/// How [`inspect_one`] reads a member record's frontier (`M`/3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierRead {
    /// The sweep: inspect the record, and read the frontier (forced) only
    /// when its seq moved past the one already read.
    Sweep,
    /// A ring change or a hint named the frontier: force-read it with no
    /// inspect first (VU3's hint contract, `RecordPort::read_hinted`): an
    /// inspect whose replica has not caught up would answer "nothing
    /// newer", and the forced read clears Veilid's report mark so the next
    /// change of it is reported.
    Hinted { minimum: Option<u64> },
    /// The claiming read lagged. Force a second fanout for the same claim
    /// under the record pool's D8 cap.
    Refetch { minimum: u64 },
}

/// Inspect `record`; when its frontier seq moved, read the frontier and
/// store every head not in `known`; the seq counts as read once all of
/// them are stored. A frontier that added nothing is checked for
/// convergence ([`check_convergence`]). Returns how many were stored.
///
/// # Errors
/// The op store failed. An unreadable record is skipped.
pub async fn inspect_one<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    record: &str,
    seen: &SeenFrontiers,
    known: &mut BTreeSet<OpHash>,
    mode: FrontierRead,
) -> Result<usize, GovernanceRuntimeError> {
    let lease = match deps.acquire_record(record, None, OpenIntent::Check).await {
        Ok(lease) => lease,
        Err(error) => {
            tracing::debug!(community = community_id, record, %error, "sweep: record did not open");
            return Ok(0);
        }
    };
    let read = match mode {
        FrontierRead::Sweep => {
            let seq = deps
                .inspect_dht_record_update_get_seqs(lease)
                .await
                .ok()
                .and_then(|seqs| {
                    seqs.get(member_record::FRONTIER as usize)
                        .copied()
                        .flatten()
                });
            // The inspect and read fan out separately. Only the value's
            // seq counts as read.
            if seq.is_some_and(|seq| seen.unread(record, seq)) {
                deps.read_hinted_value(lease, member_record::FRONTIER)
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            }
        }
        FrontierRead::Hinted { minimum } => deps
            .read_hinted_value(lease, member_record::FRONTIER)
            .await
            .ok()
            .flatten()
            .filter(|(seq, _)| minimum.is_none_or(|minimum| *seq >= minimum)),
        FrontierRead::Refetch { minimum } => match deps.record_port() {
            Ok(port) => port
                .refetch_hinted(lease, member_record::FRONTIER, minimum)
                .await
                .ok()
                .flatten()
                .map(|value| (u64::from(value.seq), value.data)),
            Err(_) => None,
        },
    };
    deps.release_record(lease).await;
    let Some((seq, bytes)) = read else {
        return Ok(0);
    };
    store_frontier_heads(deps, community_id, record, seen, known, seq, &bytes).await
}

/// Store every head of the frontier `bytes` (read at `seq`) not in
/// `known`; the seq counts as read once all of them are stored.
async fn store_frontier_heads<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    record: &str,
    seen: &SeenFrontiers,
    known: &mut BTreeSet<OpHash>,
    seq: u64,
    bytes: &[u8],
) -> Result<usize, GovernanceRuntimeError> {
    let Ok(frontier) = rekindle_codec::governance_op::decode_frontier(bytes) else {
        tracing::debug!(
            community = community_id,
            record,
            "sweep: frontier did not decode"
        );
        return Ok(0);
    };
    // The seq is recorded as read only once every head we lacked is
    // stored: a head that did not fetch keeps the frontier unread, so the
    // next inspect reads it again although its seq has not moved.
    let mut stored = 0;
    // A member's own fork first (D15, plan E3.2 S7d): its op log serves
    // one branch at the forked seq, so the other is stored from the proof,
    // verified and bounded as a gossiped op is.
    for encoded in &frontier.fork_proof {
        if let Some(hash) =
            crate::op_receive::store_received_op(deps, community_id, encoded.clone()).await?
        {
            known.insert(hash);
            stored += 1;
        }
    }
    let logs = op_log::WalkLogCache::default();
    // A fork op the proof left out (it did not fit beside the heads) is
    // fetched from the author's op log when that serves it; the log holds
    // one branch, so a miss does not keep the frontier unread.
    for op_ref in &frontier.fork_refs {
        if known.contains(&op_ref.op_hash) {
            continue;
        }
        if let Some(op) = op_log::fetch_held(deps, op_ref, &logs).await? {
            deps.op_store()
                .store_governance_op(community_id, &op)
                .await?;
            known.insert(op_ref.op_hash);
            stored += 1;
        }
    }
    let mut complete = true;
    for op_ref in &frontier.heads {
        if known.contains(&op_ref.op_hash) {
            continue;
        }
        match op_log::fetch_held(deps, op_ref, &logs).await? {
            Some(op) => {
                deps.op_store()
                    .store_governance_op(community_id, &op)
                    .await?;
                known.insert(op_ref.op_hash);
                stored += 1;
            }
            None => complete = false,
        }
    }
    if complete {
        if stored == 0 {
            check_convergence(deps, community_id, record, &frontier, seen).await?;
        }
        seen.mark_read(record, seq);
    }
    Ok(stored)
}

/// D15's convergence check (review S7m part 2 P2, part 4 Q2). Our
/// installed state is rebuilt only when its roster checksum differs from
/// the merge of our op set ([`op_log::own_frontier`]), once per op set: a
/// rebuild that did not close the lag is not repeated. A peer frontier
/// whose heads are exactly ours with another checksum is a divergence
/// signal, warned once per (record, heads, checksum) and never a rebuild:
/// rebuilding the same deterministic merge of the same op set cannot
/// change our checksum, and a peer could otherwise trigger rebuilds at
/// will. Heads that differ say nothing. The merge is cached in `seen` by
/// op-set digest.
///
/// # Errors
/// The op store failed.
async fn check_convergence<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    record: &str,
    frontier: &Frontier,
    seen: &SeenFrontiers,
) -> Result<(), GovernanceRuntimeError> {
    // A partial community's checksum is of fewer ops than its peers' (S7c.8
    // (1)): nothing to compare until the backfill ends.
    if crate::partial::is_partial(deps, community_id).await {
        return Ok(());
    }
    let Some(cert) = deps.genesis_cert(community_id) else {
        return Ok(());
    };
    // The op set and the installed state as one pair, read under the lock
    // every installer holds from its op-set read to its install (local
    // writes, the origin's genesis, rebuilds): unlocked, a state installed
    // just after our op-set read looks like a lag, and the rebuild it
    // claims spends the op set's one rebuild on nothing.
    let (held, installed) = {
        let lock = deps.rebuild_lock(community_id);
        let _installing = lock.lock().await;
        let held = deps.op_store().governance_ops(community_id).await?;
        let installed = deps
            .governance_state(community_id)
            .map(|state| roster_checksum(&state));
        (held, installed)
    };
    // No installed state: not a community we hold, nothing lags.
    let Some(installed) = installed else {
        return Ok(());
    };
    let digest = op_set_digest(held.iter().map(|op| &op.hash));
    let ours = if let Some(view) = seen.merged(&digest) {
        view
    } else {
        // Heads and checksum from one merge of the op set, as the writer's.
        let set = op_log::verify_held_with(deps, community_id, &held);
        let merged = op_log::own_frontier(&cert, &set, &held, None);
        let view = MergedView {
            heads: merged.heads.iter().map(|r| r.op_hash).collect(),
            checksum: merged.roster_checksum,
        };
        seen.cache_merged(digest, view.clone());
        view
    };
    if installed != ours.checksum {
        if seen.claim_lag_rebuild(digest) {
            tracing::info!(
                community = community_id,
                "installed governance state lags our op set's merge; rebuilding"
            );
            crate::rebuild_governance(deps, community_id).await;
        } else {
            tracing::debug!(
                community = community_id,
                "installed governance state still lags this op set after its rebuild"
            );
        }
    }
    let heads: BTreeSet<OpHash> = frontier.heads.iter().map(|r| r.op_hash).collect();
    if heads == ours.heads
        && frontier.roster_checksum != ours.checksum
        && seen.note_divergence(record, op_set_digest(&heads), frontier.roster_checksum)
    {
        tracing::warn!(
            community = community_id,
            record,
            "sweep: equal heads, different roster checksum; the member's merge diverges from ours (not rebuilt)"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "anti_entropy_tests.rs"]
mod tests;
