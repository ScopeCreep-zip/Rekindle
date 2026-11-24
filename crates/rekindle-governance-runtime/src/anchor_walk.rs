//! A fresh device's read of a community it is already a member of (plan
//! E3.2 S7d-keys K5, ADR 0011, ADR 0022 rule 4): an identity imported onto a
//! new device holds its communities in the exported identity vault but no
//! governance state, no pinned cert and no op of any of them, and it has no
//! inviter whose frontier a join would walk from (`join_walk`).
//!
//! ADR 0011 makes every op reachable from the anchor alone: the anchor pins
//! the `GenesisCert`, the cert names the creator's member record, every
//! member record's `Frontier` (`M`/3) names its member's heads, and every
//! `MemberJoined` names the joiner's member record. So the walk starts at
//! the creator's record, walks the op DAG from its frontier, and adds the
//! frontier of each member record the merged state names until a pass names
//! no record not yet read; the ops are verified and merged under the pinned
//! cert as every reader merges them. Nothing is written: the caller stores
//! the ops and enrols its own device key before any governance write.

use std::collections::BTreeSet;
use std::future::Future;

use rekindle_governance::merge::{self, MergeOutcome};
use rekindle_types::governance::{Frontier, GenesisCert, OpRef};

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::join_frontier::read_frontier;
use crate::op_log::{self, HeldOp, VerifiedSet};
use crate::op_walk;

/// What a walk from the anchor read and merged.
pub struct AnchorWalk {
    /// The pinned cert, and its bytes as the anchor holds them.
    pub cert: GenesisCert,
    pub cert_bytes: Vec<u8>,
    /// Every op fetched, for the host's op store.
    pub held: Vec<HeldOp>,
    pub set: VerifiedSet,
    pub merged: MergeOutcome,
    /// The member records whose frontier was read.
    pub records: BTreeSet<String>,
}

/// The frontiers, walk and merge of [`walk_from_anchor`] over closures:
/// `read` answers a member record's frontier (`None` for a record that
/// holds none yet), `walk` walks the DAG from heads with ops already held,
/// `verify` verifies the held ops.
///
/// # Errors
/// A frontier read or a walk failed.
pub async fn walk_records_with<R, RF, W, WF, V>(
    cert: &GenesisCert,
    read: R,
    walk: W,
    verify: V,
) -> Result<(Vec<HeldOp>, VerifiedSet, MergeOutcome, BTreeSet<String>), GovernanceRuntimeError>
where
    R: Fn(String) -> RF,
    RF: Future<Output = Result<Option<Frontier>, GovernanceRuntimeError>>,
    W: Fn(Vec<OpRef>, Vec<HeldOp>) -> WF,
    WF: Future<Output = Result<Vec<HeldOp>, GovernanceRuntimeError>>,
    V: Fn(&[HeldOp]) -> VerifiedSet,
{
    let mut records: BTreeSet<String> = BTreeSet::new();
    let mut next: Vec<String> = vec![cert.creator_member_record.clone()];
    let mut held: Vec<HeldOp> = Vec::new();
    loop {
        let mut heads: Vec<OpRef> = Vec::new();
        for record in next.drain(..) {
            if !records.insert(record.clone()) {
                continue;
            }
            if let Some(frontier) = read(record).await? {
                // A fork's ops by reference too (D15): both branches are
                // fetched as every reader of the frontier fetches them.
                heads.extend(frontier.heads);
                heads.extend(frontier.fork_refs);
            }
        }
        if !heads.is_empty() {
            held = walk(heads, held).await?;
        }
        let set = verify(&held);
        let merged = merge::merge_outcome(cert, &set.ops);
        next = (merged.state.members.values())
            .map(|m| m.member_record.clone())
            .filter(|record| !records.contains(record))
            .collect();
        if next.is_empty() {
            return Ok((held, set, merged, records));
        }
    }
}

/// Read and merge `anchor`'s op set from the anchor alone (module doc):
/// the pinned cert, then every member record's frontier the walk reaches.
///
/// # Errors
/// The anchor does not pin, or a fetch failed. A member record with no
/// readable frontier adds no heads; its ops are still reached through the
/// deps of every op that cites them.
pub async fn walk_from_anchor<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
) -> Result<AnchorWalk, GovernanceRuntimeError> {
    let (cert, cert_bytes) = crate::genesis::read_pinned(deps, anchor).await?;
    let (held, set, merged, records) = walk_records_with(
        &cert,
        |record| async move {
            match read_frontier(deps, &record).await {
                Ok(read) => Ok(Some(read.frontier)),
                Err(error) => {
                    tracing::debug!(community = anchor, %record, %error, "no frontier read");
                    Ok(None)
                }
            }
        },
        |heads, held| async move { op_walk::walk_seeded(deps, anchor, &heads, held).await },
        |held| op_log::verify_held_with(deps, anchor, held),
    )
    .await?;
    Ok(AnchorWalk {
        cert,
        cert_bytes,
        held,
        set,
        merged,
        records,
    })
}

#[cfg(test)]
#[path = "anchor_walk_tests.rs"]
mod tests;
