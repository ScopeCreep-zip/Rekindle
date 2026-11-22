//! The joiner's read of the op set (plan E3.2 S7c.5, "Using a
//! checkpoint"): from a verified hint when its inviter gives one, else the
//! full walk.
//!
//! Before its walk the joiner asks its inviter for the hints
//! (`hint_chain::verified_hints`, authorised by the invite's code hash),
//! then walks only above the newest one's cut and merges from its snapshot
//! (`hinted_walk::resume_newest`), moving to an older hint on `NotOnTop`.
//! It walks in full when the inviter gives no hint (none yet, a checkpoint
//! equivocation, an old route), no hint is on top, a resumed state misses
//! the inviter's roster checksum, the post-cut ops hold a checkpoint
//! equivocation at the hint's height, or the joiner already has an op in the
//! cut (a re-join: its next seq and its removals are in the cut, which it
//! does not hold).
//!
//! A joiner that holds ops of its own chain (a re-join after our removal:
//! the community stays as read-only history, its op set with it) walks in
//! full with them held. Its next op continues that chain, as a Matrix
//! re-join is a new `m.room.member` event in the same user's history: an
//! op of ours our op log does not hold (queued there when we were removed,
//! live walk 23 M87) would otherwise be missing from the walk, with every
//! op citing it (the removal among them), and the re-join would restart
//! our chain at seq 0, an equivocation every reader drops.

use rekindle_governance::checkpoint::CutSnapshot;
use rekindle_governance::merge::{self, MergeOutcome};
use rekindle_types::governance::{Frontier, GenesisCert};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::hint_chain::{verified_hints, Asker};
use crate::hinted_walk::{equivocates, gather, resume_newest};
use crate::op_log::{self, fetch_held, HeldOp, VerifiedSet, WalkLogCache};
use crate::op_walk;
use crate::partial::PartialJoin;

/// What the joiner read and merged.
pub struct WalkedSet {
    /// Every op fetched, for the host's op store.
    pub held: Vec<HeldOp>,
    /// The verified ops the join's deps are chosen from: the post-cut ops
    /// after a resume, every op after a full walk.
    pub set: VerifiedSet,
    pub merged: MergeOutcome,
    /// The digest of the hint the join cites (`join_deps`): the hint
    /// resumed from, else the full merge's selected hint at the greatest
    /// height.
    pub hint: Option<[u8; 32]>,
    /// The height resumed from; `None` after a full walk.
    pub resumed_at: Option<u64>,
    /// The hint resumed from: the community is partial from it until its
    /// backfill ends (plan E3.2 S7c.8 (1)); `None` after a full walk.
    pub partial: Option<PartialJoin>,
}

/// Whether `joiner` has an op in `snap`'s cut, merged or not. The cut
/// index is keyed by device key (ADR 0022 rule 1, `CutIndex.authors`,
/// `checkpoint/cut.rs`), never by pseudonym: a merged cut op names its
/// author (`CutOp.author`), and an unmerged key is `joiner`'s when the
/// snapshot's device set holds `joiner`'s endorsement of it.
fn in_the_cut(snap: &CutSnapshot, joiner: &PseudonymKey) -> bool {
    let merged = (snap.cut.authors.values()).any(|op| op.author == joiner.0);
    let ours: std::collections::BTreeSet<[u8; 32]> = (snap.state.devices.member(joiner))
        .map(|md| md.elements().into_iter().map(|e| e.device_key).collect())
        .unwrap_or_default();
    merged || snap.cut.unmerged.keys().any(|key| ours.contains(key))
}

/// The ops of `joiner`'s own chain this node holds of `anchor`.
///
/// # Errors
/// The op store could not be read.
async fn own_held<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    joiner: &PseudonymKey,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    let mut held = deps.op_store().governance_ops(anchor).await?;
    held.retain(|op| op.author == *joiner);
    Ok(held)
}

/// The full walk from `frontier`, with `own` (our own chain's held ops)
/// held from the start, and its merge.
async fn full_walk<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    cert: &GenesisCert,
    frontier: &Frontier,
    own: Vec<HeldOp>,
) -> Result<WalkedSet, GovernanceRuntimeError> {
    let held = op_walk::walk_seeded(deps, anchor, &frontier.heads, own).await?;
    let set = op_log::verify_held_with(deps, anchor, &held);
    let merged = merge::merge_outcome(cert, &set.ops);
    let hint = merged.hints.latest().map(|h| h.digest);
    Ok(WalkedSet {
        held,
        set,
        merged,
        hint,
        resumed_at: None,
        partial: None,
    })
}

/// Read and merge `anchor`'s op set from the inviter's `frontier`, from a
/// hint asked of the inviter as `asker` when one is on top (module doc).
/// `seed` holds the invite's `InviteCreated` when the frontier read was
/// older than the invite link (`join_frontier`): the walk is then full from
/// the frontier with it held, since a hint's resume compares against a
/// frontier known to be stale.
///
/// # Errors
/// A fetch failed.
pub async fn walk_for_join<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    cert: &GenesisCert,
    (frontier, seed): (&Frontier, Vec<HeldOp>),
    asker: &Asker<'_>,
) -> Result<WalkedSet, GovernanceRuntimeError> {
    let joiner = PseudonymKey(asker.me.verifying_key().to_bytes());
    let mut own = own_held(deps, anchor, &joiner).await?;
    if asker.route.is_empty() || !own.is_empty() || !seed.is_empty() {
        own.extend(seed);
        return full_walk(deps, anchor, cert, frontier, own).await;
    }
    let hints = verified_hints(deps, anchor, cert, asker).await;
    let hints: Vec<_> = hints
        .into_iter()
        .filter(|h| !in_the_cut(&h.snapshot, &joiner))
        .collect();
    let snapshots: Vec<CutSnapshot> = hints.iter().map(|h| h.snapshot.clone()).collect();
    let logs = WalkLogCache::default();
    let logs = &logs;
    let resumed = resume_newest(cert, &snapshots, frontier, |index| {
        let snap = &snapshots[index];
        gather(
            anchor,
            cert,
            snap,
            &frontier.heads,
            move |op_ref| async move { fetch_held(deps, &op_ref, logs).await },
        )
    })
    .await?;
    let Some(resumed) = resumed else {
        return full_walk(deps, anchor, cert, frontier, Vec::new()).await;
    };
    let link = &hints[resumed.index].link;
    let below = hints.get(resumed.index + 1).map(|h| &h.snapshot.state);
    if equivocates(cert, below, link.height, &resumed.gathered.rest) {
        tracing::warn!(community = %anchor, "a checkpoint equivocation: no hint, full walk");
        return full_walk(deps, anchor, cert, frontier, Vec::new()).await;
    }
    tracing::info!(
        community = %anchor,
        height = link.height,
        fetched = resumed.gathered.held.len(),
        "join resumed from a checkpoint"
    );
    Ok(WalkedSet {
        held: resumed.gathered.held,
        set: VerifiedSet {
            ops: resumed.gathered.rest,
            ..VerifiedSet::default()
        },
        merged: resumed.outcome,
        hint: Some(link.digest),
        resumed_at: Some(link.height),
        partial: Some(PartialJoin {
            height: link.height,
            digest: link.digest,
            state_root: link.body.state_root,
        }),
    })
}

#[cfg(test)]
#[path = "join_walk_tests.rs"]
mod tests;
