//! Our own `Frontier` (`M`/3, plan E3.2 S7d): the heads of the verified
//! op set, D15's roster checksum and, after a restored backup, our own
//! fork proof. Split out of [`op_log`](crate::op_log), which re-exports it.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_governance::checksum::roster_checksum;
use rekindle_governance::merge;
use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::{Frontier, GenesisCert, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::op_log::{author_record, frontier, HeldOp, VerifiedSet};

/// Our whole `Frontier` (`M`/3), heads and D15's roster checksum, both from
/// one merge of the verified set `set`: the state a rebuild of that set
/// installs ([`rebuild_ops`]). The writer publishes it and a reader
/// compares a peer's frontier against it, so a state whose rebuild is
/// still pending (the installed state behind the op store) neither
/// publishes nor compares a checksum of fewer ops than its heads name.
///
/// When `me` signed two ops at one seq (a restored backup, D15), the
/// frontier also carries both as its fork proof ([`own_fork_proof`]): our
/// op log keeps one op per seq, so a reader fetching the other branch from
/// it would never see the fork.
#[must_use]
pub fn own_frontier(
    cert: &GenesisCert,
    set: &VerifiedSet,
    held: &[HeldOp],
    me: Option<&PseudonymKey>,
) -> Frontier {
    let merged = merge::merge(cert, &set.ops);
    let fork = me.map(|me| own_fork(set, held, me)).unwrap_or_default();
    fit_fork_proof(Frontier {
        heads: frontier_refs(cert, &merged, set),
        roster_checksum: roster_checksum(&merged),
        fork_proof: fork.iter().map(|op| op.encoded.clone()).collect(),
        fork_refs: fork.iter().map(|op| op_ref_of(op)).collect(),
    })
}

/// `frontier` with its fork proof cut to what one subkey holds beside the
/// heads, the largest op left out first: two ops near `GOVERNANCE_OP_MAX_BYTES`
/// never fit one `MAX_SUBKEY_VALUE_LEN` value, and a frontier that does not
/// encode is never written, which would hide the heads too. An op left out
/// stays named in `fork_refs`; a reader fetches it from our op log, which
/// serves one branch (plan E3.2 S7d, frontier rules).
#[must_use]
pub fn fit_fork_proof(mut frontier: Frontier) -> Frontier {
    while !frontier.fork_proof.is_empty()
        && rekindle_codec::governance_op::encode_frontier(&frontier).is_err()
    {
        let largest = (0..frontier.fork_proof.len())
            .max_by_key(|&i| frontier.fork_proof[i].len())
            .unwrap_or_default();
        frontier.fork_proof.remove(largest);
    }
    frontier
}

fn op_ref_of(op: &HeldOp) -> OpRef {
    OpRef {
        op_hash: op.hash,
        device_key: op.device_key,
        author: op.author.clone(),
        author_record: op.author_record.clone(),
        author_seq: op.seq,
    }
}

/// The encoded ops `me` signed at the lowest seq it signed more than once,
/// the two lowest hashes there (D15's proof is two signed ops at one seq);
/// empty when every seq of ours holds one op.
#[must_use]
pub fn own_fork_proof(set: &VerifiedSet, held: &[HeldOp], me: &PseudonymKey) -> Vec<Vec<u8>> {
    own_fork(set, held, me)
        .into_iter()
        .map(|op| op.encoded.clone())
        .collect()
}

/// The held ops behind [`own_fork_proof`].
fn own_fork<'a>(set: &VerifiedSet, held: &'a [HeldOp], me: &PseudonymKey) -> Vec<&'a HeldOp> {
    let mut by_seq: BTreeMap<u64, BTreeSet<OpHash>> = BTreeMap::default();
    for op in set.ops.iter().filter(|op| op.author() == me) {
        by_seq.entry(op.op().seq).or_default().insert(op.hash());
    }
    let Some(fork) = by_seq.into_values().find(|hashes| hashes.len() > 1) else {
        return Vec::new();
    };
    fork.iter()
        .take(rekindle_codec::governance_op::MAX_FORK_PROOF_OPS)
        .filter_map(|hash| held.iter().find(|op| op.hash == *hash))
        .collect()
}

/// Our `Frontier` (`M`/3): the heads of the verified set as `OpRef`s, in
/// hash order; a head whose author has no member record in `state` is
/// left out.
#[must_use]
pub fn frontier_refs(cert: &GenesisCert, state: &GovernanceState, set: &VerifiedSet) -> Vec<OpRef> {
    frontier(&set.ops, None)
        .into_iter()
        .filter_map(|op| author_record(cert, state, op.author()).map(|r| op.op_ref(&r)))
        .collect()
}
