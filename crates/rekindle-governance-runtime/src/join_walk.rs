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

use rekindle_governance::checkpoint::CutSnapshot;
use rekindle_governance::merge::{self, MergeOutcome};
use rekindle_types::governance::{Frontier, GenesisCert};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::hint_chain::{verified_hints, Asker};
use crate::hinted_walk::{equivocates, gather, resume_newest};
use crate::op_log::{self, fetch_held, HeldOp, VerifiedSet, WalkLogCache};

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
}

/// Whether `joiner` has an op in `snap`'s cut, merged or not.
fn in_the_cut(snap: &CutSnapshot, joiner: &PseudonymKey) -> bool {
    snap.cut.authors.contains_key(&joiner.0) || snap.cut.unmerged.contains_key(&joiner.0)
}

/// The full walk from `frontier` and its merge.
async fn full_walk<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    cert: &GenesisCert,
    frontier: &Frontier,
) -> Result<WalkedSet, GovernanceRuntimeError> {
    let held = op_log::walk(deps, anchor, &frontier.heads).await?;
    let set = op_log::verify_held_with(deps, anchor, &held);
    let merged = merge::merge_outcome(cert, &set.ops);
    let hint = merged.hints.latest().map(|h| h.digest);
    Ok(WalkedSet {
        held,
        set,
        merged,
        hint,
        resumed_at: None,
    })
}

/// Read and merge `anchor`'s op set from the inviter's `frontier`, from a
/// hint asked of the inviter as `asker` when one is on top (module doc).
///
/// # Errors
/// A fetch failed.
pub async fn walk_for_join<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    cert: &GenesisCert,
    frontier: &Frontier,
    asker: &Asker<'_>,
) -> Result<WalkedSet, GovernanceRuntimeError> {
    if asker.route.is_empty() {
        return full_walk(deps, anchor, cert, frontier).await;
    }
    let joiner = PseudonymKey(asker.me.verifying_key().to_bytes());
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
        return full_walk(deps, anchor, cert, frontier).await;
    };
    let link = &hints[resumed.index].link;
    let below = hints.get(resumed.index + 1).map(|h| &h.snapshot.state);
    if equivocates(cert, below, link.height, &resumed.gathered.rest) {
        tracing::warn!(community = %anchor, "a checkpoint equivocation: no hint, full walk");
        return full_walk(deps, anchor, cert, frontier).await;
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
    })
}
