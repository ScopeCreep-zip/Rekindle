//! `merge_from`'s gates over the post-cut ops continue from the cut index
//! (plan E3.2 S7c.4a): masks, record holders, invite codes, re-joins.

use rekindle_types::governance::GovernanceEntry;

use super::super::op_proptests::{find, head, joined, kick, pk};
use super::super::{NotOnTop, QuarantineReason};
use super::{base, resume};
use crate::fixture::{meta, op};

/// A re-join after a removal in the cut is exempt only while the re-join
/// applies in the round (`gates::rejoined`), so the full merge's fixpoint
/// depends on its earlier rounds: `merge_from` answers `Undecidable`.
#[test]
fn a_rejoin_after_a_cut_removal_is_undecidable() {
    let (cert, mut ops) = base();
    let k = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 40, kick(4, 1));
    ops.push(k);
    let back = op(
        &pk(4),
        1,
        &[head(&ops), find(&ops, 4, 0)],
        41,
        joined(4, Some("code")),
    );
    let hash = back.hash();
    ops.push(back);
    let heads = [&ops[ops.len() - 2]];
    assert_eq!(
        resume(&cert, &ops, &heads).err(),
        Some(NotOnTop::Undecidable(hash))
    );
}

#[test]
fn a_member_kicked_in_the_cut_stays_masked_after_it() {
    let (cert, mut ops) = base();
    let k = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 40, kick(4, 1));
    let cause = k.hash();
    ops.push(k);
    let late = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 41, meta("late"));
    let masked = late.hash();
    ops.push(late);
    let heads = [&ops[ops.len() - 2]];
    let resumed = resume(&cert, &ops, &heads).expect("on top");
    let reason = QuarantineReason::AfterAuthorRemoval { cause };
    assert_eq!(resumed.quarantined, vec![(masked, reason)]);
}

#[test]
fn a_join_naming_a_record_held_before_the_cut_is_refused() {
    let (cert, mut ops) = base();
    let GovernanceEntry::MemberJoined {
        invite_proof,
        owner_proof,
        member_record_owner,
        ..
    } = joined(9, Some("code"))
    else {
        unreachable!("a join");
    };
    let taken = GovernanceEntry::MemberJoined {
        member_record: "VLD0:m2".into(),
        member_record_owner,
        invite_proof,
        owner_proof,
    };
    let j = op(&pk(9), 0, &[head(&ops)], 40, taken);
    let hash = j.hash();
    ops.push(j);
    let heads = [&ops[ops.len() - 2]];
    let resumed = resume(&cert, &ops, &heads).expect("on top");
    // Quarantined with a visible reason (D15), not silently refused.
    assert!(resumed.quarantined.iter().any(|(h, reason)| *h == hash
        && matches!(reason, crate::merge::QuarantineReason::RecordClaimed { .. })));
    assert!(!resumed.refused.contains(&hash));
}

#[test]
fn a_join_on_an_invite_revoked_in_the_cut_is_refused() {
    let (cert, mut ops) = base();
    let revoke = GovernanceEntry::InviteRevoked { invite_id: [7; 16] };
    let r = op(&pk(1), 6, &[head(&ops)], 40, revoke);
    ops.push(r);
    let j = op(&pk(9), 0, &[head(&ops)], 41, joined(9, Some("code")));
    let hash = j.hash();
    ops.push(j);
    let heads = [&ops[ops.len() - 2]];
    let resumed = resume(&cert, &ops, &heads).expect("on top");
    assert!(resumed.refused.contains(&hash));
}

/// SU1 + S7c.8 (MR1, seed 2330924744309632130): a revoke beside the cut,
/// concurrent with a cut join on its invite, refuses that join in the full
/// merge; the snapshot counts the cut's joins but cannot name them, so the
/// resume is not on top rather than keeping the joiner a member.
#[test]
fn a_revoke_beside_the_cut_concurrent_with_a_cut_join_is_not_on_top() {
    let (cert, mut ops) = base();
    let before = head(&ops).clone();
    let j = op(&pk(6), 0, &[find(&ops, 1, 1)], 40, joined(6, Some("code")));
    let revoke = GovernanceEntry::InviteRevoked { invite_id: [7; 16] };
    let r = op(&pk(1), 6, &[&before], 41, revoke);
    let (joined_hash, revoke_hash) = (j.hash(), r.hash());
    ops.push(j);
    ops.push(r);
    let full = super::super::merge_outcome(&cert, &ops);
    assert!(
        full.refused.contains(&joined_hash),
        "outside the revoke's past"
    );
    let heads = [&before, &ops[ops.len() - 2]];
    assert_eq!(
        resume(&cert, &ops, &heads).err(),
        Some(NotOnTop::BesideTheCut(revoke_hash))
    );
}

/// Plan E3.2 S7s.3: the checkpoint-resume approval gate
/// (`resume_gates::approval_gate`) passes a further creator's post-cut ops
/// without an approval, as the full merge does (decision (17)): limited
/// to the primary creator, the resume refuses the further creator's ops
/// and its state departs from the full merge's.
#[test]
fn a_further_creator_passes_the_resume_approval_gate() {
    use rekindle_types::governance::AdmissionMode;
    let mut cert = crate::fixture::cert(&pk(1), AdmissionMode::ApprovalRequired);
    cert.additional_creators = vec![pk(7)];
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, meta("before"));
    let f0 = op(&pk(7), 0, &[&c1], 2, joined(7, None));
    let f1 = op(&pk(7), 1, &[&f0], 3, meta("by a further creator"));
    let late = f1.hash();
    let ops = vec![c0, c1, f0, f1];
    let heads = [&ops[1]];
    let resumed = resume(&cert, &ops, &heads).expect("on top");
    assert!(
        resumed.applied.contains(&late),
        "the further creator's op applies"
    );
    assert!(resumed.state.is_counted_member(&pk(7)));
    assert!(!resumed.state.pending_members.contains_key(&pk(7)));
}
