//! Test fixtures: op sets built from `(lamport, entry)` lists without
//! signing, for this crate's tests and its benches (feature `fixture`).
//! Production ops come only from [`crate::op::VerifiedOp::verify`].

use rekindle_types::governance::{
    AdmissionMode, GenesisCert, GovernanceEntry, GovernanceOp, OpHash, OpRef,
};
use rekindle_types::id::PseudonymKey;

use crate::op::{OpStamp, VerifiedOp};
use crate::state::GovernanceState;

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
    }
}

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

/// One fixture op: `deps` as given, hash from `tag`.
#[must_use]
pub fn op(
    author: &PseudonymKey,
    seq: u64,
    deps: &[&VerifiedOp],
    tag: u64,
    entry: GovernanceEntry,
) -> VerifiedOp {
    let prev = deps
        .iter()
        .find(|d| d.author() == author && d.op().seq + 1 == seq)
        .map(|d| d.hash());
    let lamport = 1 + deps.iter().map(|d| d.lamport()).max().unwrap_or(0);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&author.0);
    hasher.update(&seq.to_le_bytes());
    hasher.update(&tag.to_le_bytes());
    let hash = OpHash(*hasher.finalize().as_bytes());
    let op = GovernanceOp {
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
    let mut all: Vec<(u64, &PseudonymKey, usize, &GovernanceEntry)> = Vec::new();
    for (author, entries) in subkeys {
        for (i, (lamport, entry)) in entries.iter().enumerate() {
            all.push((*lamport, author, i, entry));
        }
    }
    all.sort_by(|a, b| (a.0, &a.1 .0, a.2).cmp(&(b.0, &b.1 .0, b.2)));
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
        let given = op(author, *seq, &deps, tag as u64, entry.clone());
        let next =
            VerifiedOp::assume_verified(given.op().clone(), given.hash(), lamport.max(floor));
        *seq += 1;
        out.push(next);
    }
    out
}

/// Merge with the creator the test meant: the author of the first entry,
/// its `AdmissionPolicy` (if any) as the cert's policy.
#[must_use]
pub fn merge_entries(subkeys: &[(PseudonymKey, Vec<Stamped>)]) -> GovernanceState {
    let ops = ops_in_order(subkeys);
    let Some(first) = ops.first() else {
        return GovernanceState::default();
    };
    let creator = first.author().clone();
    let policy = ops
        .iter()
        .find_map(|o| match o.body() {
            GovernanceEntry::AdmissionPolicy { mode } if o.author() == &creator => Some(*mode),
            _ => None,
        })
        .unwrap_or_default();
    crate::merge::merge(&cert(&creator, policy), &ops)
}
