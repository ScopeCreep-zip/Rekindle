//! Time-of-op judgement and per-author strong removal (plan E3.2 S7c.3a):
//! a non-power op is judged against the power state in its own causal
//! past, and a removal masks exactly its target's ops at or above the seq
//! frontier it carries.

use rekindle_types::governance::{AdmissionMode, Checkpoint, GovernanceEntry, ModReportTarget};

use super::super::{merge, merge_outcome};
use super::{
    assign, community, find, head, invite, joined, kick, pk, quarantined_by, role, MergeOutcome,
};
use crate::checkpoint::{eligible_set_hash, frontier_hash};
use crate::fixture::{cert, meta, op};
use crate::invite_quota::MAX_USES_PER_INVITE;
use crate::op::VerifiedOp;

fn report(id: u8) -> GovernanceEntry {
    GovernanceEntry::ModReport {
        report_id: [id; 16],
        target: ModReportTarget::Member { pseudonym: pk(1) },
        reason: "spam".into(),
    }
}

fn ban(target: u8, frontier: u64) -> GovernanceEntry {
    GovernanceEntry::BanEntry {
        target: pk(target),
        reason: None,
        target_seq_frontier: frontier,
    }
}

/// A height-1 checkpoint over `cut`.
fn checkpoint(cut: &[&VerifiedOp]) -> GovernanceEntry {
    let heads: Vec<_> = cut.iter().map(|o| o.hash()).collect();
    GovernanceEntry::Checkpoint(Checkpoint {
        height: 1,
        round: 0,
        prev: None,
        cut: cut.iter().map(|o| o.op_ref("VLD0:fixture")).collect(),
        frontier_hash: frontier_hash(&heads),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: [0; 32],
        mek_generation: 1,
        eligible_set_hash: eligible_set_hash(&[pk(1)]),
        pages: Vec::new(),
    })
}

fn stands(outcome: &MergeOutcome, o: &VerifiedOp) -> bool {
    outcome.applied.contains(&o.hash())
        && !outcome.refused.contains(&o.hash())
        && outcome.quarantined.iter().all(|(h, _)| *h != o.hash())
}

/// T's ops at seqs below the removal's frontier stand, judged at their
/// own position: member 4's checkpoint (members only) before its kick, and
/// manager 3's rename before its ban, though neither is a member, nor
/// unbanned, in the final state.
#[test]
fn time_of_op_judgement_keeps_a_kicked_members_prior_ops() {
    let mut ops = community();
    let d1 = op(
        &pk(4),
        1,
        &[head(&ops), find(&ops, 4, 0)],
        30,
        checkpoint(&[head(&ops)]),
    );
    let k = op(&pk(2), 1, &[&d1, find(&ops, 2, 0)], 31, kick(4, 2));
    let b1 = op(&pk(3), 1, &[&k, find(&ops, 3, 0)], 32, meta("by three"));
    let ban = op(&pk(1), 6, &[&b1], 33, ban(3, 2));
    ops.extend([d1.clone(), k.clone(), b1.clone(), ban.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(stands(&outcome, &k) && stands(&outcome, &ban));
    assert!(!outcome.state.members.contains_key(&pk(4)), "kicked");
    assert!(outcome.state.bans.contains(&pk(3)), "banned");
    assert!(
        stands(&outcome, &d1),
        "the kicked member's prior checkpoint"
    );
    assert!(stands(&outcome, &b1), "the banned manager's prior rename");
    let name = outcome.state.metadata.as_ref().map(|m| m.name.as_str());
    assert_eq!(name, Some("by three"));
}

/// An op of a kicked member citing only deps from before the kick still
/// takes its next seq, at the kick's frontier: it is masked and
/// quarantined with the kick as its cause, not refused.
#[test]
fn a_backdated_op_after_a_kick_is_masked() {
    let mut ops = community();
    let k = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 30, kick(4, 1));
    let backdated = op(&pk(4), 1, &[find(&ops, 4, 0)], 31, report(9));
    ops.extend([k.clone(), backdated.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(stands(&outcome, &k));
    assert!(quarantined_by(&outcome, &backdated, &k));
    assert!(!outcome.refused.contains(&backdated.hash()));
    assert!(!outcome.applied.contains(&backdated.hash()));
    assert!(outcome.state.mod_reports.is_empty());
}

/// A removal's seq frontier is a pure function of its deps: a kick of
/// member 4 (one op, `d0`, in its past) claiming 0 or 2 is refused and
/// masks nothing; claiming 1, it applies.
#[test]
fn a_removal_whose_seq_frontier_differs_from_its_deps_is_refused() {
    for (claimed, holds) in [(0, false), (2, false), (1, true)] {
        let mut ops = community();
        let k = op(
            &pk(2),
            1,
            &[head(&ops), find(&ops, 2, 0)],
            30,
            kick(4, claimed),
        );
        let after = op(&pk(4), 1, &[&k, find(&ops, 4, 0)], 31, report(9));
        ops.extend([k.clone(), after.clone()]);
        let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
        assert_eq!(
            outcome.applied.contains(&k.hash()),
            holds,
            "claimed {claimed}"
        );
        assert_eq!(
            outcome.refused.contains(&k.hash()),
            !holds,
            "claimed {claimed}"
        );
        assert_eq!(outcome.state.members.contains_key(&pk(4)), !holds);
        assert_eq!(quarantined_by(&outcome, &after, &k), holds);
        assert_eq!(outcome.applied.contains(&after.hash()), !holds);
    }
}

/// Manager 3 renames the community while it holds role 5; the creator
/// then takes the role away, having seen the rename. The rename is judged
/// at its own position and stands; the demotion masks only what follows.
#[test]
fn a_later_power_op_never_re_judges_an_earlier_non_power_op() {
    let mut ops = community();
    let b1 = op(
        &pk(3),
        1,
        &[head(&ops), find(&ops, 3, 0)],
        30,
        meta("by three"),
    );
    let demote = op(
        &pk(1),
        6,
        &[&b1],
        31,
        GovernanceEntry::RoleUnassignment {
            target: pk(3),
            role_id: rekindle_types::id::RoleId([5; 16]),
        },
    );
    let b2 = op(&pk(3), 2, &[&demote, &b1], 32, meta("too late"));
    ops.extend([b1.clone(), demote.clone(), b2.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(stands(&outcome, &demote));
    assert!(stands(&outcome, &b1), "judged before the demotion");
    assert!(outcome.refused.contains(&b2.hash()), "after the demotion");
    let name = outcome.state.metadata.as_ref().map(|m| m.name.as_str());
    assert_eq!(name, Some("by three"));
    // A later role definition changes nothing about either.
    let c7 = op(&pk(1), 7, &[&b2], 33, role(9, 1));
    ops.push(c7);
    let later = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(stands(&later, &b1));
}

/// Under `ApprovalRequired`, a joiner's op signed before its approval is
/// refused and one signed after it applies: `admitted` is read at each
/// op's anchor, not in the final state.
#[test]
fn a_pending_members_op_before_its_approval_is_refused_and_one_after_it_applies() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(MAX_USES_PER_INVITE));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
    let before = op(&pk(2), 1, &[&a0], 3, report(1));
    let approve = op(
        &pk(1),
        2,
        &[&before, &c1],
        4,
        GovernanceEntry::MemberApproved { target: pk(2) },
    );
    let after = op(&pk(2), 2, &[&approve, &before], 5, report(2));
    let ops = [c0, c1, a0, before.clone(), approve, after.clone()];
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::ApprovalRequired), &ops);
    assert!(outcome.state.members.contains_key(&pk(2)), "approved");
    assert!(outcome.refused.contains(&before.hash()), "before approval");
    assert!(stands(&outcome, &after), "after approval");
    assert!(outcome.state.mod_reports.contains_key(&[2; 16]));
    assert!(!outcome.state.mod_reports.contains_key(&[1; 16]));
}

#[test]
fn approval_required_holds_joiners_pending() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(MAX_USES_PER_INVITE));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
    let b0 = op(&pk(3), 0, &[&c1], 3, joined(3, Some("code")));
    let a1 = op(&pk(2), 1, &[&a0, &b0], 4, assign(2, 6));
    let approve = op(
        &pk(1),
        2,
        &[&a1],
        5,
        GovernanceEntry::MemberApproved { target: pk(2) },
    );
    let reject = op(
        &pk(1),
        3,
        &[&approve],
        6,
        GovernanceEntry::MemberRejected {
            target: pk(3),
            reason: None,
        },
    );
    let cert = cert(&pk(1), AdmissionMode::ApprovalRequired);
    let pending = merge(&cert, &[c0.clone(), c1.clone(), a0.clone(), b0.clone()]);
    assert!(pending.pending_members.contains_key(&pk(2)));
    assert!(!pending.members.contains_key(&pk(2)));
    let outcome = merge_outcome(&cert, &[c0, c1, a0, b0, a1.clone(), approve, reject]);
    let state = &outcome.state;
    assert!(state.members.contains_key(&pk(2)), "approved");
    assert!(!state.members.contains_key(&pk(3)), "rejected");
    assert!(!state.pending_members.contains_key(&pk(3)));
    // Signed before its author's approval: refused at its causal position.
    assert!(outcome.refused.contains(&a1.hash()));
}
