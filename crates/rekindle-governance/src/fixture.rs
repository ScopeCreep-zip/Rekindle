//! Test fixtures: op sets built from `(lamport, entry)` lists without
//! signing, for this crate's tests and its benches (feature `fixture`).
//! Production ops come only from [`crate::op::VerifiedOp::verify`].

use rekindle_types::governance::{
    AdmissionMode, GenesisCert, GovernanceEntry, GovernanceOp, OpHash, OpRef,
};
use rekindle_types::id::PseudonymKey;

use crate::op::{OpStamp, VerifiedOp};
use crate::state::GovernanceState;

/// The merge's units of work, machine-independent, for the tests that gate
/// its complexity on counts rather than on wall clock
/// (`tests/perf_targets.rs`). Kept only in test and `fixture` builds.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MergeCounts {
    /// Fixpoint rounds (`fixpoint::settle` passes), over every resolution.
    pub rounds: u64,
    /// `try_apply` calls, each one `validate_write`.
    pub try_apply: u64,
    /// `Ancestry::contains` calls.
    pub contains: u64,
    /// Ancestry rows built (`build_through`).
    pub rows_built: u64,
    /// `u64` words OR'd into ancestry rows (`build_through`).
    pub words_ored: u64,
    /// Ops examined by `frontier_holds` counting a removal's target ops.
    pub frontier_steps: u64,
}

std::thread_local! {
    static COUNTS: std::cell::Cell<MergeCounts> = const {
        std::cell::Cell::new(MergeCounts {
            rounds: 0,
            try_apply: 0,
            contains: 0,
            rows_built: 0,
            words_ored: 0,
            frontier_steps: 0,
        })
    };
}

/// The work `run` makes the merge do on this thread.
pub fn counts(run: impl FnOnce()) -> MergeCounts {
    COUNTS.with(|c| c.set(MergeCounts::default()));
    run();
    COUNTS.with(std::cell::Cell::get)
}

/// Adds to this thread's counts (the crate's `tally!`).
pub(crate) fn tally(add: impl FnOnce(&mut MergeCounts)) {
    COUNTS.with(|c| {
        let mut counts = c.get();
        add(&mut counts);
        c.set(counts);
    });
}

/// An entry with the position the test wants it ordered at.
pub type Stamped = (u64, GovernanceEntry);

#[must_use]
pub fn at(lamport: u64, entry: GovernanceEntry) -> Stamped {
    (lamport, entry)
}

/// A stamp at `lamport` for hand-built state.
#[must_use]
pub fn stamp(lamport: u64) -> OpStamp {
    OpStamp {
        lamport,
        hash: OpHash([0; 32]),
    }
}

/// The `code_hash` of the invite whose code is `code`: its public join key
/// (plan E3.2 S10), the only form a reader admits.
#[must_use]
pub fn code_key(code: &str) -> String {
    rekindle_secrets::invite::hash_invite_code(code)
}

/// A `CommunityMeta` entry naming the community `name`.
#[must_use]
pub fn meta(name: &str) -> GovernanceEntry {
    GovernanceEntry::CommunityMeta {
        name: Some(name.into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
    }
}

/// A cert naming `creator`, with `policy`.
#[must_use]
pub fn cert(creator: &PseudonymKey, policy: AdmissionMode) -> GenesisCert {
    GenesisCert {
        name: "fixture".into(),
        policy,
        initial_roles: Vec::new(),
        creator_pseudonym: creator.clone(),
        creator_member_record: "VLD0:creator-record".into(),
        creator_member_record_owner: "creator-owner".into(),
        created_at: 0,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: None,
    }
}

/// The community anchor every fixture op is verified for.
pub const FIXTURE_ANCHOR: &str = "VLD0:anchor";

/// A fixture op claiming `lamport` outright (a forger's view of itself).
#[must_use]
pub fn op_at(
    author: &PseudonymKey,
    seq: u64,
    deps: &[&VerifiedOp],
    tag: u64,
    lamport: u64,
    entry: GovernanceEntry,
) -> VerifiedOp {
    let given = op(author, seq, deps, tag, entry);
    VerifiedOp::assume_verified(given.op().clone(), given.hash(), lamport)
}

/// One fixture op: `deps` as given, hash from `tag`, signed (in name) by
/// the author's own key as its device key.
#[must_use]
pub fn op(
    author: &PseudonymKey,
    seq: u64,
    deps: &[&VerifiedOp],
    tag: u64,
    entry: GovernanceEntry,
) -> VerifiedOp {
    op_on(author, author.0, seq, deps, tag, entry)
}

/// One fixture op of `author` at `seq` on `device_key`'s chain (ADR 0022
/// rule 1: a seq numbers its device key's chain, so `prev` is that key's
/// op at `seq - 1`).
#[must_use]
pub fn op_on(
    author: &PseudonymKey,
    device_key: [u8; 32],
    seq: u64,
    deps: &[&VerifiedOp],
    tag: u64,
    entry: GovernanceEntry,
) -> VerifiedOp {
    let prev = deps
        .iter()
        .find(|d| d.op().device_key == device_key && d.op().seq + 1 == seq)
        .map(|d| d.hash());
    let lamport = 1 + deps.iter().map(|d| d.lamport()).max().unwrap_or(0);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&author.0);
    hasher.update(&device_key);
    hasher.update(&seq.to_le_bytes());
    hasher.update(&tag.to_le_bytes());
    let hash = OpHash(*hasher.finalize().as_bytes());
    let op = GovernanceOp {
        device_key,
        author: author.clone(),
        seq,
        prev,
        deps: deps
            .iter()
            .map(|d| d.op_ref("VLD0:fixture"))
            .collect::<Vec<OpRef>>(),
        issued_at: lamport,
        body: entry,
        sig: [0; 64],
    };
    VerifiedOp::assume_verified(op, hash, lamport)
}

/// A total chain in `(lamport, author)` order: each op depends on the one
/// before it.
#[must_use]
pub fn ops_in_order(subkeys: &[(PseudonymKey, Vec<Stamped>)]) -> Vec<VerifiedOp> {
    // Ties at one (lamport, author) break on the entry itself, so the order
    // an author's entries are listed in never changes the chain.
    let mut all: Vec<(u64, &PseudonymKey, String, &GovernanceEntry)> = Vec::new();
    for (author, entries) in subkeys {
        for (lamport, entry) in entries {
            all.push((*lamport, author, format!("{entry:?}"), entry));
        }
    }
    all.sort_by(|a, b| (a.0, &a.1 .0, &a.2).cmp(&(b.0, &b.1 .0, &b.2)));
    let mut seqs: std::collections::HashMap<&PseudonymKey, u64> = std::collections::HashMap::new();
    let mut out: Vec<VerifiedOp> = Vec::new();
    for (tag, (lamport, author, _, entry)) in all.into_iter().enumerate() {
        let seq = seqs.entry(author).or_insert(0);
        let mut deps: Vec<&VerifiedOp> = out.last().into_iter().collect();
        if let Some(prev) = out.iter().rev().find(|o| o.author() == author) {
            if deps.iter().all(|d| d.hash() != prev.hash()) {
                deps.push(prev);
            }
        }
        let floor = out.last().map_or(1, |d| d.lamport().saturating_add(1));
        // A removal's seq frontier, as a writer fills it: the chain puts
        // every earlier op in its past (plan E3.2 S7c.3a).
        let entry = match crate::power::removal_target(entry) {
            Some(target) => crate::power::with_target_seq_frontier(
                entry.clone(),
                out.iter().filter(|o| o.author() == target).count() as u64,
            ),
            None => entry.clone(),
        };
        let given = op(author, *seq, &deps, tag as u64, entry);
        let next =
            VerifiedOp::assume_verified(given.op().clone(), given.hash(), lamport.max(floor));
        *seq += 1;
        out.push(next);
    }
    out
}

/// Merge under a cert naming `creator` (default admission mode): the
/// creator comes from the cert alone, never from an op's position.
#[must_use]
pub fn merge_entries(
    creator: &PseudonymKey,
    subkeys: &[(PseudonymKey, Vec<Stamped>)],
) -> GovernanceState {
    merge_entries_under(&cert(creator, AdmissionMode::default()), subkeys)
}

/// Merge under `cert`.
#[must_use]
pub fn merge_entries_under(
    cert: &GenesisCert,
    subkeys: &[(PseudonymKey, Vec<Stamped>)],
) -> GovernanceState {
    crate::merge::merge(cert, &ops_in_order(subkeys))
}

/// `ops` without the ones `drop` picks, each survivor's deps re-pointed
/// past them (a dropped op's deps stand in for it); hashes and lamports
/// are kept, so the survivors are the same ops as in `ops`.
#[must_use]
pub fn without(ops: &[VerifiedOp], drop: impl Fn(&VerifiedOp) -> bool) -> Vec<VerifiedOp> {
    let mut dropped: std::collections::HashMap<OpHash, Vec<OpRef>> =
        std::collections::HashMap::new();
    let mut out = Vec::new();
    for o in ops {
        let mut deps: Vec<OpRef> = Vec::new();
        for d in &o.op().deps {
            match dropped.get(&d.op_hash) {
                Some(through) => deps.extend(through.iter().cloned()),
                None => deps.push(d.clone()),
            }
        }
        deps.dedup_by_key(|d| d.op_hash);
        if drop(o) {
            dropped.insert(o.hash(), deps);
            continue;
        }
        let mut op = o.op().clone();
        op.deps = deps;
        out.push(VerifiedOp::assume_verified(op, o.hash(), o.lamport()));
    }
    out
}
