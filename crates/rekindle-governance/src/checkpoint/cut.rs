//! The cut index and `stateRoot` (plan E3.2 S7c.3 part (2)): what a reader
//! needs, beside the merged state, to skip the cut's ops and reproduce every
//! post-cut fate. O(N + power ops), never O(history).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_types::domains;
use rekindle_types::governance::{OpHash, OpRef};

use crate::checksum::canon::{put_map, put_set, Canon};
use crate::checksum::canonical_state;
use crate::checksum::decode::{CanonReader, Decanon};
use crate::op::VerifiedOp;
use crate::state::GovernanceState;

/// A cut op as the cut index names it: its hash, author (pseudonym key
/// bytes) and seq, so a reader holding only the snapshot locates it in
/// its author's op log through the directory at the cut (member →
/// record, `CutSnapshot::locate`) plus its seq (plan E3.2 S7c.5, P3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CutOp {
    pub hash: OpHash,
    pub author: [u8; 32],
    pub seq: u64,
}

impl CutOp {
    /// `op`'s entry.
    #[must_use]
    pub fn of(op: &VerifiedOp) -> Self {
        Self {
            hash: op.hash(),
            author: op.author().0,
            seq: op.op().seq,
        }
    }
}

impl Canon for CutOp {
    fn put(&self, out: &mut Vec<u8>) {
        self.hash.put(out);
        self.author.put(out);
        self.seq.put(out);
    }
}

impl Decanon for CutOp {
    fn take(r: &mut CanonReader<'_>) -> Option<Self> {
        Some(Self {
            hash: Decanon::take(r)?,
            author: Decanon::take(r)?,
            seq: Decanon::take(r)?,
        })
    }
}

/// The cut index of one cut.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CutIndex {
    /// Each author's highest seq in the cut, with that op's hash, keyed by
    /// pseudonym bytes: an op is in the cut iff its seq is at or below its
    /// author's entry.
    pub authors: BTreeMap<[u8; 32], (u64, OpHash)>,
    /// The cut's power ops in the order the power phase visits them, each
    /// with its fate (`true`: applied, `false`: refused), located by author
    /// and seq (S7c.5, P3): `merge_from` needs every one of them.
    pub power: Vec<(CutOp, bool)>,
    /// The cut's power ops no other cut power op has in its causal past.
    pub power_heads: Vec<OpHash>,
    /// The greatest mainline key `(issued_at, op_hash)` among the cut's
    /// non-power ops, if it has any.
    pub mainline_max: Option<(u64, OpHash)>,
    /// `record_gate`'s holders after the cut: each member record claimed
    /// by an applied join or move, with the pseudonym (key bytes) holding
    /// it (plan E3.2 S7c.4a: the gate continues from them, as
    /// `invite_gate` continues from the stored `uses`).
    pub holders: BTreeMap<String, [u8; 32]>,
    /// The authors whose later ops the cut's removals mask (an applied
    /// `MemberRemoved`, `BanEntry` or demotion of them that no applied
    /// re-join in the cut follows), each with the first such removal
    /// carrying a seq frontier, the quarantine cause (plan E3.2 S7c.4a).
    pub masked: BTreeMap<[u8; 32], Option<OpHash>>,
    /// Per invite code hash, the first applied `InviteCreated` in the cut
    /// carrying it, whether an applied `InviteRevoked` in the cut names
    /// it, and that `InviteCreated`'s locator: `invite_gate` reads the
    /// first such invite in a join's past (plan E3.2 S7c.4a), and a join
    /// beside the cut has it in its past only when its deps reach the
    /// locator's author at or above its seq (S7c.8).
    pub invite_codes: BTreeMap<String, ([u8; 16], bool, CutOp)>,
    /// The authors that signed two ops at one seq in the cut, each with the
    /// lowest such seq: every op of theirs at or above it is dropped on
    /// every reader (`resolve::prepare`), later ones included, and
    /// `authors` counts only the ops below it (plan E3.2 S7c.4a).
    pub forks: BTreeMap<[u8; 32], u64>,
    /// The authors with cut ops the merge leaves out (pending, or at or
    /// above their fork), each with the highest seq of any of its cut ops:
    /// an op at a seq above its author's `authors` entry and at or below
    /// this one is in the cut but never merged (plan E3.2 S7c.4a).
    pub unmerged: BTreeMap<[u8; 32], u64>,
    /// The lamport of each author's op in `authors` (plan E3.2 S7c.5): a
    /// post-cut op's lamport is one above its deps', so a reader that
    /// skipped the cut verifies an op citing an author's top cut op from
    /// this (`VerifiedOp::verify_with`). Every cut head is such an op.
    pub lamports: BTreeMap<[u8; 32], u64>,
    /// The cut's applied `Checkpoint` ops at its two greatest heights,
    /// located by author and seq, with their heights (plan E3.2 S7c.5): a
    /// post-cut `Checkpoint` naming one as `prevCheckpoint` (the
    /// endorsements of the hint whose cut this is name one at the height
    /// below) is judged from it, and the hint chain check fetches them.
    pub checkpoints: BTreeMap<CutOp, u64>,
}

impl CutIndex {
    /// The height of the cut's applied `Checkpoint` op `hash`, when
    /// `checkpoints` lists it.
    #[must_use]
    pub fn checkpoint_height(&self, hash: &OpHash) -> Option<u64> {
        (self.checkpoints.iter())
            .find(|(c, _)| c.hash == *hash)
            .map(|(_, h)| *h)
    }

    /// What verifying a post-cut op reads of its dep `r` when `r` is its
    /// author's top op in the cut (`authors`); `None` for any other op.
    #[must_use]
    pub fn top_facts<'a>(&self, r: &'a OpRef) -> Option<crate::op::DepFacts<'a>> {
        let (seq, hash) = self.authors.get(&r.author.0)?;
        if *seq != r.author_seq || *hash != r.op_hash {
            return None;
        }
        Some(crate::op::DepFacts {
            author: &r.author,
            // The op hash matched above covers the op's device key, so the
            // locator's is the op's.
            device_key: &r.device_key,
            seq: *seq,
            lamport: *self.lamports.get(&r.author.0)?,
        })
    }
}

/// The canonical encoding of `cut`, under `canonical_state`'s rules.
#[must_use]
pub fn canonical_cut(cut: &CutIndex) -> Vec<u8> {
    let CutIndex {
        authors,
        power,
        power_heads,
        mainline_max,
        holders,
        masked,
        invite_codes,
        forks,
        unmerged,
        lamports,
        checkpoints,
    } = cut;
    let mut out = Vec::new();
    put_map(&mut out, authors);
    power.put(&mut out);
    put_set(&mut out, power_heads);
    mainline_max.put(&mut out);
    put_map(&mut out, holders);
    put_map(&mut out, masked);
    put_map(&mut out, invite_codes);
    put_map(&mut out, forks);
    put_map(&mut out, unmerged);
    put_map(&mut out, lamports);
    put_map(&mut out, checkpoints);
    out
}

/// The preimage of a checkpoint's `stateRoot`, `canonical_state(state) ‖
/// canonical_cut(cut)`: the bytes a state snapshot carries (plan E3.2
/// S7c.3, "State snapshot"). The `Frontier`'s roster checksum hashes the
/// first part alone, so the two share one encoder while the cut index,
/// which differs between readers holding different op sets, stays out of
/// D15.
#[must_use]
pub fn state_preimage(state: &GovernanceState, cut: &CutIndex) -> Vec<u8> {
    let mut out = canonical_state(state);
    out.extend_from_slice(&canonical_cut(cut));
    out
}

/// BLAKE3 `derive_key(GOV_CHECKPOINT_STATE)` over a `stateRoot` preimage,
/// which a reader checks a reassembled snapshot with before decoding it.
#[must_use]
pub fn state_root_of(preimage: &[u8]) -> [u8; 32] {
    blake3::derive_key(domains::GOV_CHECKPOINT_STATE, preimage)
}

/// A checkpoint's `stateRoot`: [`state_root_of`] its [`state_preimage`].
#[must_use]
pub fn state_root(state: &GovernanceState, cut: &CutIndex) -> [u8; 32] {
    state_root_of(&state_preimage(state, cut))
}

/// The ops `frontier` reaches through deps (and `prev`), the frontier
/// included, in the set's order: a cut's ops when `frontier` is its heads.
/// A frontier op not held is left out, as is everything only it reaches.
#[must_use]
pub fn causal_past(ops: &[VerifiedOp], frontier: &[OpRef]) -> Vec<VerifiedOp> {
    let by_hash: HashMap<OpHash, &VerifiedOp> = ops.iter().map(|o| (o.hash(), o)).collect();
    let mut reached: BTreeSet<OpHash> = BTreeSet::new();
    let mut stack: Vec<OpHash> = frontier.iter().map(|r| r.op_hash).collect();
    while let Some(hash) = stack.pop() {
        let Some(op) = by_hash.get(&hash) else {
            continue;
        };
        if reached.insert(hash) {
            stack.extend(op.op().deps.iter().map(|d| d.op_hash));
            stack.extend(op.op().prev);
        }
    }
    ops.iter()
        .filter(|o| reached.contains(&o.hash()))
        .cloned()
        .collect()
}
