//! The joiner's walk above a checkpoint's cut (plan E3.2 S7c.5, "Using a
//! checkpoint"). With a verified hint and its snapshot, the walk from the
//! inviter's frontier stops at the cut: every ref at or below its author's
//! highest seq in the cut (merged or not, `CutIndex.authors`/`unmerged`)
//! is not walked into. What the merge from the snapshot still needs of the
//! cut is fetched by locator, and nothing else:
//!
//! - the cut ops the post-cut ops cite, and the cut's applied power ops
//!   (`CutSnapshot::power_refs`, which `merge_from` folds) and the refused
//!   ones the post-cut ops cite (a join is dated against them, S7c.8), with
//!   the cut ops they cite (their deps, `prev` among them), each checked for its hash,
//!   its author's signature, its locator's author and seq, and a seq inside
//!   the cut ([`cut_fact`]). Their own in-cut deps are not fetched: the
//!   strictly endorsed snapshot vouches for the cut (the step's
//!   *Decided 2026-11-17* note);
//! - post-cut ops are verified against those facts ([`verify_over_cut`]).
//!
//! **A cited cut op's lamport.** No op carries its lamport (it is `1 +
//! max(dep lamports)`, `rekindle_governance::op`), so a cut op other than
//! its author's top cannot be given its own without its in-cut past. The
//! cut index gives each top's (`CutIndex.lamports`, every head among them);
//! any other cut op is given the greatest of those, the greatest head
//! lamport, which bounds every cut op's lamport from above (each cut op is
//! in some head's causal past). That makes the lamport of every op on top
//! of the cut exact: it has every head in its causal past (`merge_from`'s
//! (a′)), so one of its deps is a head or a post-cut op above one, and that
//! dep's lamport is at least the bound. An op that is not on top makes
//! `merge_from` answer `NotOnTop`, and the joiner moves on.
//!
//! An op **beside** the cut (plan E3.2 S7c.8 (3): a join signed
//! concurrently with the checkpoint cites the old base) may be on top
//! without a head in its past, so the bound would inflate its lamport,
//! which its state rows carry (a join's `joined` stamp). Each non-top cut
//! op a post-cut op cites is therefore given its exact lamport from its
//! in-cut past, fetched by locator down to ops the cut index gives a
//! lamport for (authors' tops) or with no deps ([`exact_lamports`]); a
//! post-cut op on top keeps the lamport the bound gave it. A cut op whose
//! past cannot be fetched and checked leaves the snapshot to full replay,
//! as an unlocatable cut power op does.
//!
//! [`resume_newest`] tries the verified hints newest first: `NotOnTop`
//! moves to the next older one, none on top leaves the full replay to the
//! caller. A resumed state is checked against the inviter's `Frontier`
//! roster checksum as D15 does ([`ring_holds`]): at equal heads, a
//! mismatch drops the hint and the caller replays in full (a colluding
//! majority can endorse a false snapshot, threat model T8).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;

use futures::stream::{self, StreamExt, TryStreamExt};
use rekindle_codec::governance_op::decode_op;
use rekindle_governance::checkpoint::{eligible_set, is_strict_majority, tally, CutSnapshot};
use rekindle_governance::checksum::roster_checksum;
use rekindle_governance::merge::{merge_from, MergeOutcome};
use rekindle_governance::op::{op_hash, op_signing_bytes, DepFacts, OpRejection, VerifiedOp};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::sign::verify_signature;
use rekindle_types::governance::{
    Checkpoint, Frontier, GenesisCert, GovernanceEntry, GovernanceOp, OpHash, OpRef,
};
use rekindle_types::id::PseudonymKey;

use crate::error::GovernanceRuntimeError;
use crate::op_log::HeldOp;
use crate::op_walk::WALK_FETCHES_IN_FLIGHT;

/// Where a ref sits against the cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Above its author's cut ops: walked.
    Above,
    /// At or below its author's highest merged cut seq: fetched by locator
    /// when cited.
    Merged,
    /// In the cut but never merged (pending, or at or above a fork): the
    /// op citing it waits, as it does in the full merge.
    Unmerged,
}

fn place(snap: &CutSnapshot, r: &OpRef) -> Place {
    let author = &r.author.0;
    if (snap.cut.authors.get(author)).is_some_and(|(seq, _)| r.author_seq <= *seq) {
        Place::Merged
    } else if (snap.cut.unmerged.get(author)).is_some_and(|seq| r.author_seq <= *seq) {
        Place::Unmerged
    } else {
        Place::Above
    }
}

/// Whether `r` names an op at or below its author's highest merged seq in
/// the cut.
#[must_use]
pub fn in_cut(snap: &CutSnapshot, r: &OpRef) -> bool {
    place(snap, r) == Place::Merged
}

fn deps_of(held: &HeldOp) -> Vec<OpRef> {
    decode_op(&held.encoded).map_or_else(|_| Vec::new(), |d| d.op.deps)
}

/// Fetch `refs`, at most [`WALK_FETCHES_IN_FLIGHT`] at once, keeping the
/// ones found.
async fn fetch_each<F, Fut>(
    refs: Vec<OpRef>,
    fetch: &F,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError>
where
    F: Fn(OpRef) -> Fut,
    Fut: Future<Output = Result<Option<HeldOp>, GovernanceRuntimeError>>,
{
    stream::iter(refs.into_iter().map(fetch))
        .buffer_unordered(WALK_FETCHES_IN_FLIGHT)
        .try_filter_map(|op| async move { Ok(op) })
        .try_collect()
        .await
}

/// Walk from `start` through deps above `snap`'s cut: the post-cut ops
/// found, and the merged cut ops they cite, by hash.
///
/// # Errors
/// A fetch failed.
pub async fn walk_above<F, Fut>(
    snap: &CutSnapshot,
    start: &[OpRef],
    fetch: &F,
) -> Result<(Vec<HeldOp>, BTreeMap<OpHash, OpRef>), GovernanceRuntimeError>
where
    F: Fn(OpRef) -> Fut,
    Fut: Future<Output = Result<Option<HeldOp>, GovernanceRuntimeError>>,
{
    let mut rest: Vec<HeldOp> = Vec::new();
    let mut cited: BTreeMap<OpHash, OpRef> = BTreeMap::new();
    let mut tried: BTreeSet<OpHash> = BTreeSet::new();
    let mut wanted: Vec<OpRef> = start.to_vec();
    loop {
        let mut level = Vec::new();
        for r in wanted {
            match place(snap, &r) {
                Place::Merged => {
                    cited.entry(r.op_hash).or_insert(r);
                }
                Place::Above if tried.insert(r.op_hash) => level.push(r),
                Place::Above | Place::Unmerged => {}
            }
        }
        if level.is_empty() {
            return Ok((rest, cited));
        }
        let fetched = fetch_each(level, fetch).await?;
        wanted = fetched.iter().flat_map(deps_of).collect();
        rest.extend(fetched);
    }
}

/// What a dependant's verification reads of a fetched cut op: its author,
/// seq and lamport (module doc: a top's from the cut index, any other's
/// the greatest top lamport).
#[derive(Debug, Default, Clone)]
pub struct CutFacts(HashMap<OpHash, (PseudonymKey, u64, u64)>);

impl CutFacts {
    fn get(&self, hash: &OpHash) -> Option<DepFacts<'_>> {
        self.0.get(hash).map(|(author, seq, lamport)| DepFacts {
            author,
            seq: *seq,
            lamport: *lamport,
        })
    }

    /// The hashes of the cut ops these facts hold.
    #[must_use]
    pub fn hashes(&self) -> BTreeSet<OpHash> {
        self.0.keys().copied().collect()
    }
}

/// The facts of the cut op `held`, fetched for the locator `r` of
/// community `anchor`: `None` unless it decodes with `r`'s hash, its
/// author's signature verifies, its author and seq are `r`'s, and its seq
/// is at or below its author's highest merged seq in the cut.
#[must_use]
pub fn cut_fact(
    anchor: &str,
    snap: &CutSnapshot,
    r: &OpRef,
    held: &HeldOp,
) -> Option<(OpHash, (PseudonymKey, u64, u64))> {
    let decoded = decode_op(&held.encoded).ok()?;
    let op = &decoded.op;
    let hash = op_hash(&decoded.unsigned, &op.sig);
    if hash != r.op_hash || op.author != r.author || op.seq != r.author_seq {
        return None;
    }
    verify_signature(
        &op.author.0,
        &op_signing_bytes(anchor, &decoded.unsigned),
        &op.sig,
    )
    .ok()?;
    let (top_seq, top_hash) = snap.cut.authors.get(&op.author.0)?;
    if op.seq > *top_seq {
        return None;
    }
    let lamport = if (op.seq, hash) == (*top_seq, *top_hash) {
        *snap.cut.lamports.get(&op.author.0)?
    } else {
        snap.cut.lamports.values().copied().max()?
    };
    Some((hash, (op.author.clone(), op.seq, lamport)))
}

/// Verify `held` (post-cut ops, or the cut's power ops) of community
/// `anchor` in dependency order, each dep resolved among the ones verified
/// so far or `facts`: the ones that verify. An op still short of a dep or
/// refused is left out, as `verify_held` leaves it out of the merge.
#[must_use]
pub fn verify_over_cut(anchor: &str, facts: &CutFacts, held: &[HeldOp]) -> Vec<VerifiedOp> {
    let mut waiting: Vec<(GovernanceOp, Vec<u8>)> = (held.iter())
        .filter_map(|h| {
            let d = decode_op(&h.encoded).ok()?;
            (op_hash(&d.unsigned, &d.op.sig) == h.hash).then_some((d.op, d.unsigned))
        })
        .collect();
    let mut verified: Vec<VerifiedOp> = Vec::new();
    let mut index: HashMap<OpHash, usize> = HashMap::new();
    loop {
        let mut still = Vec::new();
        let before = verified.len();
        for (op, unsigned) in waiting {
            let result = VerifiedOp::verify_with(anchor, op.clone(), &unsigned, |h| {
                (index.get(h).map(|&i| verified[i].facts())).or_else(|| facts.get(h))
            });
            match result {
                Ok(v) if !index.contains_key(&v.hash()) => {
                    index.insert(v.hash(), verified.len());
                    verified.push(v);
                }
                Err(OpRejection::MissingDep(_)) => still.push((op, unsigned)),
                Ok(_) | Err(_) => {}
            }
        }
        if verified.len() == before || still.is_empty() {
            return verified;
        }
        waiting = still;
    }
}

/// What one hint needs for `merge_from`, and every op fetched for it.
#[derive(Debug, Default)]
pub struct Gathered {
    /// The cut's applied power ops, verified over their cited cut ops.
    pub power: Vec<VerifiedOp>,
    /// The post-cut ops, verified.
    pub rest: Vec<VerifiedOp>,
    /// Every op fetched (post-cut and cut), once each.
    pub held: Vec<HeldOp>,
    /// The cut ops fetched by locator and checked.
    pub cut_fetched: BTreeSet<OpHash>,
}

/// Gather what `merge_from` needs above `snap`'s cut for community
/// `anchor`, from `start` (the inviter's frontier) through `fetch`: `None`
/// when a cut power op cannot be located (its author has no record at the
/// cut: such a snapshot replays in full, as the step records).
///
/// # Errors
/// A fetch failed.
pub async fn gather<F, Fut>(
    anchor: &str,
    cert: &GenesisCert,
    snap: &CutSnapshot,
    start: &[OpRef],
    fetch: F,
) -> Result<Option<Gathered>, GovernanceRuntimeError>
where
    F: Fn(OpRef) -> Fut,
    Fut: Future<Output = Result<Option<HeldOp>, GovernanceRuntimeError>>,
{
    let Some(power_refs) = snap.power_refs(cert) else {
        return Ok(None);
    };
    let (rest_held, mut cited) = walk_above(snap, start, &fetch).await?;
    let rest_cited: BTreeSet<OpHash> = cited.keys().copied().collect();
    let refused_cited = (cited.values())
        .filter(|r| (snap.cut.power.iter()).any(|(c, applied)| !applied && c.hash == r.op_hash));
    let power_refs = power_refs
        .into_iter()
        .chain(refused_cited.cloned())
        .collect();
    let power_held = fetch_each(power_refs, &fetch).await?;
    for dep in power_held.iter().flat_map(deps_of) {
        if in_cut(snap, &dep) {
            cited.entry(dep.op_hash).or_insert(dep);
        }
    }
    let cut_held = fetch_each(cited.values().cloned().collect(), &fetch).await?;
    let mut facts = CutFacts(
        (cut_held.iter())
            .filter_map(|h| cut_fact(anchor, snap, cited.get(&h.hash)?, h))
            .collect(),
    );
    let roots: Vec<&HeldOp> = (cut_held.iter())
        .filter(|h| rest_cited.contains(&h.hash) && facts.0.contains_key(&h.hash))
        .collect();
    let Some((exact, below)) = exact_lamports(anchor, snap, &roots, &fetch).await? else {
        return Ok(None);
    };
    for (hash, (_, _, lamport)) in &mut facts.0 {
        if let Some(l) = exact.get(hash) {
            *lamport = *l;
        }
    }
    let power = verify_over_cut(anchor, &facts, &power_held);
    let rest = verify_over_cut(anchor, &facts, &rest_held);
    let mut seen = BTreeSet::new();
    let held = (rest_held
        .into_iter()
        .chain(power_held)
        .chain(cut_held)
        .chain(below))
    .filter(|h| seen.insert(h.hash))
    .collect();
    Ok(Some(Gathered {
        power,
        rest,
        held,
        cut_fetched: facts.hashes(),
    }))
}

/// The exact lamports of the cut ops `roots` (fetched and checked), with
/// the cut ops fetched below them: each one's in-cut past is fetched by
/// locator, checked as [`cut_fact`] checks, down to authors' tops (whose
/// lamports the cut index gives) and ops with no deps. `None` when an op
/// of that past is missing, fails its check, or cites an op outside the
/// cut's merged ops.
///
/// # Errors
/// A fetch failed.
async fn exact_lamports<F, Fut>(
    anchor: &str,
    snap: &CutSnapshot,
    roots: &[&HeldOp],
    fetch: &F,
) -> Result<Option<(HashMap<OpHash, u64>, Vec<HeldOp>)>, GovernanceRuntimeError>
where
    F: Fn(OpRef) -> Fut,
    Fut: Future<Output = Result<Option<HeldOp>, GovernanceRuntimeError>>,
{
    let mut known: HashMap<OpHash, u64> = (snap.cut.authors.iter())
        .filter_map(|(author, (_, hash))| Some((*hash, *snap.cut.lamports.get(author)?)))
        .collect();
    let mut deps: HashMap<OpHash, Vec<OpHash>> = HashMap::new();
    let mut below = Vec::new();
    let mut want: Vec<OpRef> = Vec::new();
    for held in roots.iter().filter(|h| !known.contains_key(&h.hash)) {
        let cites = deps_of(held);
        deps.insert(held.hash, cites.iter().map(|r| r.op_hash).collect());
        want.extend(cites);
    }
    loop {
        let mut seen = BTreeSet::new();
        want.retain(|r| {
            !known.contains_key(&r.op_hash)
                && !deps.contains_key(&r.op_hash)
                && seen.insert(r.op_hash)
        });
        if want.is_empty() {
            break;
        }
        if !want.iter().all(|r| in_cut(snap, r)) {
            return Ok(None);
        }
        let by_hash: HashMap<OpHash, OpRef> = want.iter().map(|r| (r.op_hash, r.clone())).collect();
        let got = fetch_each(std::mem::take(&mut want), fetch).await?;
        if got.len() != by_hash.len() {
            return Ok(None);
        }
        for held in got {
            let r = by_hash.get(&held.hash);
            if r.and_then(|r| cut_fact(anchor, snap, r, &held)).is_none() {
                return Ok(None);
            }
            let cites = deps_of(&held);
            deps.insert(held.hash, cites.iter().map(|r| r.op_hash).collect());
            want.extend(cites);
            below.push(held);
        }
    }
    let mut exact = HashMap::new();
    for root in roots {
        let Some(l) = lamport_of(root.hash, &deps, &mut known) else {
            return Ok(None);
        };
        exact.insert(root.hash, l);
    }
    Ok(Some((exact, below)))
}

/// `1 + max(dep lamports)` of `root` over `deps`, memoised in `known`
/// (iterative, post-order).
fn lamport_of(
    root: OpHash,
    deps: &HashMap<OpHash, Vec<OpHash>>,
    known: &mut HashMap<OpHash, u64>,
) -> Option<u64> {
    let mut stack = vec![(root, false)];
    while let Some((hash, expanded)) = stack.pop() {
        if known.contains_key(&hash) {
            continue;
        }
        let cites = deps.get(&hash)?;
        if expanded {
            let lamports: Option<Vec<u64>> = cites.iter().map(|d| known.get(d).copied()).collect();
            known.insert(hash, 1 + lamports?.into_iter().max().unwrap_or(0));
        } else {
            stack.push((hash, true));
            stack.extend(
                cites
                    .iter()
                    .filter(|d| !known.contains_key(d))
                    .map(|d| (*d, false)),
            );
        }
    }
    known.get(&root).copied()
}

/// D15 for a resumed `state`: when every head of the inviter's `frontier`
/// is held (a verified post-cut op, or a merged cut op), the heads are
/// equal and the state's roster checksum must be the frontier's; with a
/// head not held the two are not comparable and the check holds.
#[must_use]
pub fn ring_holds(
    snap: &CutSnapshot,
    rest: &[VerifiedOp],
    state: &GovernanceState,
    frontier: &Frontier,
) -> bool {
    let held: BTreeSet<OpHash> = rest.iter().map(VerifiedOp::hash).collect();
    let equal_heads = (frontier.heads.iter()).all(|h| held.contains(&h.op_hash) || in_cut(snap, h));
    !equal_heads || roster_checksum(state) == frontier.roster_checksum
}

/// Whether the post-cut ops `rest` hold a checkpoint equivocation at
/// `height` (the height of the hint resumed from, whose endorsements are
/// above its cut): two digests or more for one cut and one
/// `prevCheckpoint`, each carried by a strict majority of the electorate
/// `below` leaves (the state at the height below's cut; the cert's
/// creator set at height 1). A reader then uses no hint (plan E3.2 S7c.4).
#[must_use]
pub fn equivocates(
    cert: &GenesisCert,
    below: Option<&GovernanceState>,
    height: u64,
    rest: &[VerifiedOp],
) -> bool {
    let electorate = eligible_set(cert, below);
    let mut groups: BTreeMap<Vec<u8>, Vec<(&PseudonymKey, &Checkpoint)>> = BTreeMap::new();
    for op in rest {
        if let GovernanceEntry::Checkpoint(c) = op.body() {
            if c.height == height {
                let key: Vec<u8> = (c.cut.iter().map(|r| r.op_hash.0))
                    .chain(c.prev.iter().map(|r| r.op_hash.0))
                    .flatten()
                    .collect();
                groups.entry(key).or_default().push((op.author(), c));
            }
        }
    }
    groups.into_values().any(|votes| {
        let majorities = tally(&electorate, votes)
            .values()
            .filter(|endorsers| is_strict_majority(endorsers.len(), electorate.len()))
            .count();
        majorities > 1
    })
}

/// A merge resumed from a hint.
#[derive(Debug)]
pub struct Resumed {
    /// The hint's position in the list tried (0: the newest).
    pub index: usize,
    pub outcome: MergeOutcome,
    pub gathered: Gathered,
}

/// Resume from the first of `snapshots` (newest first) the ops `gather`
/// returns for it are on top of: `NotOnTop` moves to the next older one.
/// `None` (the caller replays in full): none is on top, a snapshot's cut
/// power op cannot be located, or the resumed state misses `frontier`'s
/// roster checksum at equal heads ([`ring_holds`]).
///
/// # Errors
/// A fetch failed.
pub async fn resume_newest<G, Fut>(
    cert: &GenesisCert,
    snapshots: &[CutSnapshot],
    frontier: &Frontier,
    gather: G,
) -> Result<Option<Resumed>, GovernanceRuntimeError>
where
    G: Fn(usize) -> Fut,
    Fut: Future<Output = Result<Option<Gathered>, GovernanceRuntimeError>>,
{
    for (index, snap) in snapshots.iter().enumerate() {
        let Some(gathered) = gather(index).await? else {
            return Ok(None);
        };
        match merge_from(cert, snap, &gathered.power, &gathered.rest) {
            Ok(outcome) if ring_holds(snap, &gathered.rest, &outcome.state, frontier) => {
                return Ok(Some(Resumed {
                    index,
                    outcome,
                    gathered,
                }));
            }
            Ok(_) => {
                tracing::warn!(
                    index,
                    "a resumed state misses the roster checksum: full replay"
                );
                return Ok(None);
            }
            Err(why) => tracing::debug!(index, ?why, "not on top of the hint: an older one"),
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "hinted_walk_tests.rs"]
mod tests;
