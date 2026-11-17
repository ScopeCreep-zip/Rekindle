//! The checkpoint op (plan E3.2 S7c.3): `stateRoot` over the merged cut,
//! the causal cut rules, the electorate and endorsement counting.

use rekindle_types::governance::{AdmissionMode, Checkpoint, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::op_proptests::{community, head, joined, kick, pk, role};
use super::{merge_cut, merge_outcome};
use crate::checkpoint::{
    check_shape, checkpoint_digest, eligible_set, eligible_set_hash, frontier_hash,
    is_strict_majority, state_root, tally,
};
use crate::checksum::{canonical_state, roster_checksum};
use crate::fixture::{cert, op};
use crate::op::{OpRejection, VerifiedOp};

fn body(height: u64, prev: Option<&VerifiedOp>, cut: &[&VerifiedOp]) -> Checkpoint {
    let heads: Vec<OpHash> = cut.iter().map(|o| o.hash()).collect();
    Checkpoint {
        height,
        round: 0,
        prev: prev.map(|p| p.op_ref("VLD0:fixture")),
        cut: cut.iter().map(|o| o.op_ref("VLD0:fixture")).collect(),
        frontier_hash: frontier_hash(&heads),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: [0; 32],
        mek_generation: 1,
        eligible_set_hash: eligible_set_hash(&[pk(1)]),
        pages: Vec::new(),
    }
}

fn root(ops: &[VerifiedOp]) -> [u8; 32] {
    let (state, cut) = merge_cut(&cert(&pk(1), AdmissionMode::Open), ops);
    state_root(&state, &cut)
}

/// `community()` plus one op by `author` at `seq` on its head.
fn plus(author: u8, seq: u64, entry: GovernanceEntry) -> Vec<VerifiedOp> {
    let mut ops = community();
    let next = op(&pk(author), seq, &[head(&ops)], 90, entry);
    ops.push(next);
    ops
}

#[test]
fn the_state_root_is_equal_across_arrival_orders() {
    let ops = plus(2, 1, kick(4));
    let mut reversed = ops.clone();
    reversed.reverse();
    assert_eq!(root(&ops), root(&reversed));
}

#[test]
fn the_state_root_changes_on_a_ban_a_removal_a_role_or_an_invite_use() {
    let ban = GovernanceEntry::BanEntry {
        target: pk(4),
        reason: None,
    };
    let roots = [
        root(&community()),
        root(&plus(1, 6, ban)),
        root(&plus(2, 1, kick(4))),
        root(&plus(1, 6, role(7, 1))),
        root(&plus(5, 0, joined(5, Some("code")))),
    ];
    for (i, a) in roots.iter().enumerate() {
        for b in &roots[i + 1..] {
            assert_ne!(a, b, "change {i} left the state root as it was");
        }
    }
}

#[test]
fn the_state_root_and_the_roster_checksum_hash_one_encoding() {
    let (state, cut) = merge_cut(&cert(&pk(1), AdmissionMode::Open), &community());
    let mut h = blake3::Hasher::new_derive_key(rekindle_types::domains::GOV_CHECKPOINT_STATE);
    h.update(&canonical_state(&state));
    h.update(&super::super::checkpoint::canonical_cut(&cut));
    assert_eq!(state_root(&state, &cut), *h.finalize().as_bytes());
    let mut r = blake3::Hasher::new_derive_key(rekindle_types::domains::GOV_ROSTER_CHECKSUM);
    r.update(&canonical_state(&state));
    assert_eq!(roster_checksum(&state).0, *r.finalize().as_bytes());
}

/// A checkpoint is a hint: a removal after its cut merges as it would with
/// no checkpoint, and the checkpoint itself changes no state.
#[test]
fn a_removal_arriving_after_the_cut_applies_normally() {
    let mut ops = community();
    let cp = op(
        &pk(1),
        6,
        &[head(&ops)],
        91,
        GovernanceEntry::Checkpoint(body(1, None, &[head(&ops)])),
    );
    let removal = op(&pk(2), 1, &[&cp], 92, kick(4));
    let without = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops).state;
    ops.extend([cp.clone(), removal.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(outcome.applied.contains(&cp.hash()));
    assert!(outcome.applied.contains(&removal.hash()));
    assert!(!outcome.state.members.contains_key(&pk(4)));
    let mut kicked_only = without;
    kicked_only.members.remove(&pk(4));
    assert_eq!(kicked_only.members, outcome.state.members);
}

#[test]
fn a_checkpoint_whose_deps_lack_a_cut_head_is_refused() {
    let ops = community();
    let other = &ops[4];
    let mut bad = body(1, None, &[head(&ops), other]);
    let cp = op(
        &pk(1),
        6,
        &[head(&ops)],
        93,
        GovernanceEntry::Checkpoint(bad.clone()),
    );
    assert_eq!(check_shape(cp.op()), Err(OpRejection::BadCheckpoint));
    bad.cut.truncate(1);
    bad.frontier_hash = frontier_hash(&[head(&ops).hash()]);
    let good = op(
        &pk(1),
        6,
        &[head(&ops)],
        93,
        GovernanceEntry::Checkpoint(bad),
    );
    assert_eq!(check_shape(good.op()), Ok(()));
}

/// The second checkpoint's cut must hold the first's; a non-member's
/// checkpoint is refused.
#[test]
fn a_cut_without_the_previous_cut_is_refused() {
    let mut ops = community();
    let c5 = head(&ops).clone();
    let cp1 = op(
        &pk(1),
        6,
        &[&c5],
        94,
        GovernanceEntry::Checkpoint(body(1, None, &[&c5])),
    );
    // pk(4)'s seq 1 on its join only: c5 is not in its past.
    let behind = op(&pk(4), 1, &[&ops[4]], 95, joined(4, None));
    let ahead = op(&pk(3), 1, &[&c5], 96, role(8, 1));
    let cp2_bad = op(
        &pk(1),
        7,
        &[&cp1, &behind],
        97,
        GovernanceEntry::Checkpoint(body(2, Some(&cp1), &[&behind])),
    );
    let cp2_good = op(
        &pk(2),
        1,
        &[&cp1, &ahead],
        98,
        GovernanceEntry::Checkpoint(body(2, Some(&cp1), &[&ahead])),
    );
    let stranger = op(
        &pk(9),
        0,
        &[&c5],
        99,
        GovernanceEntry::Checkpoint(body(1, None, &[&c5])),
    );
    ops.extend([
        cp1.clone(),
        behind,
        ahead,
        cp2_bad.clone(),
        cp2_good.clone(),
        stranger.clone(),
    ]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(outcome.applied.contains(&cp1.hash()));
    assert!(
        outcome.refused.contains(&cp2_bad.hash()),
        "the previous cut is not inside"
    );
    assert!(outcome.applied.contains(&cp2_good.hash()));
    assert!(outcome.refused.contains(&stranger.hash()), "not a member");
}

#[test]
fn the_first_eligible_set_is_the_creator_set() {
    assert_eq!(
        eligible_set(&cert(&pk(1), AdmissionMode::Open), None),
        vec![pk(1)]
    );
}

#[test]
fn the_eligible_set_is_taken_at_the_previous_checkpoints_cut() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let (at_cut, _) = merge_cut(&c, &community());
    assert_eq!(
        eligible_set(&c, Some(&at_cut)),
        (1..=4).map(pk).collect::<Vec<_>>()
    );
    let (after_kick, _) = merge_cut(&c, &plus(2, 1, kick(4)));
    assert_eq!(
        eligible_set(&c, Some(&after_kick)),
        (1..=3).map(pk).collect::<Vec<_>>()
    );
}

#[test]
fn endorsements_count_once_per_member() {
    let electorate: Vec<PseudonymKey> = (1..=4).map(pk).collect();
    let mut a = body(1, None, &[]);
    a.eligible_set_hash = eligible_set_hash(&electorate);
    let mut again = a.clone();
    again.round = 1;
    again.pages = vec!["VLD0:page".into()];
    let (p1, p2) = (pk(1), pk(2));
    let votes = tally(&electorate, [(&p1, &a), (&p1, &again), (&p2, &a)]);
    let endorsers = &votes[&checkpoint_digest(&a)];
    assert_eq!(votes.len(), 1, "round and pages are outside the digest");
    assert_eq!(endorsers.len(), 2);
    assert!(
        !is_strict_majority(endorsers.len(), electorate.len()),
        "2 of 4 is no majority"
    );
    assert!(is_strict_majority(3, 4));
}

#[test]
fn an_old_set_signature_never_counts_toward_a_new_set() {
    let old: Vec<PseudonymKey> = (1..=3).map(pk).collect();
    let new: Vec<PseudonymKey> = (1..=4).map(pk).collect();
    let mut signed_for_old = body(1, None, &[]);
    signed_for_old.eligible_set_hash = eligible_set_hash(&old);
    let (p1, p9) = (pk(1), pk(9));
    assert!(tally(&new, [(&p1, &signed_for_old)]).is_empty());
    assert_eq!(tally(&old, [(&p1, &signed_for_old)]).len(), 1);
    let mut stranger = signed_for_old.clone();
    stranger.eligible_set_hash = eligible_set_hash(&new);
    assert!(
        tally(&new, [(&p9, &stranger)]).is_empty(),
        "outside the electorate"
    );
}
