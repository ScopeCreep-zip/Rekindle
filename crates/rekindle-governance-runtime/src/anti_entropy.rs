//! Governance anti-entropy (plan E3.2 "Watching", `evidence/slot-integrity.md`
//! §4.3 (iii)): every [`SWEEP_PERIOD`] a member inspects [`SWEEP_SAMPLE`]
//! random member records and, on login or reconnect, all of them. Each
//! inspected record's `Frontier` (`M`/3) seq is compared with the one last
//! read; a changed frontier is read and every head we do not hold is fetched
//! by its `OpRef` and stored. The rebuild that follows merges them, which
//! records an equivocating author's ops as `Equivocation` (D15): a forked
//! chain shows up as two ops cited by different frontiers. A frontier whose
//! heads equal ours carries the roster checksum its member merged; one that
//! differs from ours logs a WARN and rebuilds our state (D15's convergence
//! check, [`check_convergence`]). These records are
//! deliberately unwatched by most members (the ring watches 4 of them), so
//! this is the one app inspect D4 allows.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rekindle_governance::checksum::roster_checksum;
use rekindle_records::port::OpenIntent;
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{Frontier, OpHash};

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::op_log;

/// Member records inspected per periodic sweep.
pub const SWEEP_SAMPLE: usize = 16;
/// The periodic sweep's period.
pub const SWEEP_PERIOD: Duration = Duration::from_secs(60);

/// The frontier seq last read per member record, per community.
pub type SeenFrontiers = BTreeMap<String, u64>;

/// Which records a sweep inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// Every admitted member's record and the creator's (login, reconnect).
    Full,
    /// [`SWEEP_SAMPLE`] of them, chosen at random.
    Sample,
}

/// The member records of `community_id`'s merged state plus the creator's,
/// in record order.
fn member_records<D: GovernanceRuntimeDeps>(deps: &D, community_id: &str) -> Vec<String> {
    let mut records: BTreeSet<String> = deps
        .governance_state(community_id)
        .map(|s| {
            s.members
                .values()
                .map(|m| m.member_record.clone())
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
    seen: &mut SeenFrontiers,
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
        .governance_ops(community_id)
        .await?
        .iter()
        .map(|op| op.hash)
        .collect();
    let mut stored = 0;
    for record in &records {
        stored += inspect_one(deps, community_id, record, seen, &mut known).await?;
    }
    Ok(stored)
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
    seen: &mut SeenFrontiers,
    known: &mut BTreeSet<OpHash>,
) -> Result<usize, GovernanceRuntimeError> {
    let lease = match deps.acquire_record(record, None, OpenIntent::Check).await {
        Ok(lease) => lease,
        Err(error) => {
            tracing::debug!(community = community_id, record, %error, "sweep: record did not open");
            return Ok(0);
        }
    };
    let seq = deps
        .inspect_dht_record_update_get_seqs(lease)
        .await
        .ok()
        .and_then(|seqs| {
            seqs.get(member_record::FRONTIER as usize)
                .copied()
                .flatten()
        });
    let read = match seq {
        Some(seq) if seen.get(record) != Some(&seq) => {
            let bytes = deps
                .get_dht_value(lease, member_record::FRONTIER, true)
                .await;
            Some((seq, bytes))
        }
        _ => None,
    };
    deps.release_record(lease).await;
    let Some((seq, Ok(Some(bytes)))) = read else {
        return Ok(0);
    };
    let Ok(frontier) = rekindle_codec::governance_op::decode_frontier(&bytes) else {
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
    let mut complete = true;
    for op_ref in &frontier.heads {
        if known.contains(&op_ref.op_hash) {
            continue;
        }
        match op_log::fetch_held(deps, op_ref).await? {
            Some(op) => {
                deps.store_governance_op(community_id, &op).await?;
                known.insert(op_ref.op_hash);
                stored += 1;
            }
            None => complete = false,
        }
    }
    if complete {
        if stored == 0 {
            check_convergence(deps, community_id, record, &frontier).await?;
        }
        seen.insert(record.to_owned(), seq);
    }
    Ok(stored)
}

/// D15's convergence check: when `frontier`'s heads are exactly the heads
/// we publish, its roster checksum must equal the one of our merged state;
/// a mismatch is logged and our state rebuilt from the op set
/// ([`crate::rebuild_governance`]). Heads that differ say nothing.
///
/// # Errors
/// The op store failed.
async fn check_convergence<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    record: &str,
    frontier: &Frontier,
) -> Result<(), GovernanceRuntimeError> {
    let Some(cert) = deps.genesis_cert(community_id) else {
        return Ok(());
    };
    let state = deps.governance_state(community_id).unwrap_or_default();
    let held = deps.governance_ops(community_id).await?;
    let set = op_log::verify_held(community_id, &held);
    let ours: BTreeSet<OpHash> = op_log::frontier_refs(&cert, &state, &set)
        .iter()
        .map(|r| r.op_hash)
        .collect();
    let theirs: BTreeSet<OpHash> = frontier.heads.iter().map(|r| r.op_hash).collect();
    if ours != theirs {
        return Ok(());
    }
    if roster_checksum(&state) == frontier.roster_checksum {
        return Ok(());
    }
    tracing::warn!(
        community = community_id,
        record,
        "sweep: equal heads, different roster checksum; rebuilding the governance state"
    );
    crate::rebuild_governance(deps, community_id).await;
    Ok(())
}

#[cfg(test)]
#[path = "anti_entropy_tests.rs"]
mod tests;
