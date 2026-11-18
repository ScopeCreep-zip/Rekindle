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
    assert!(resumed.refused.contains(&hash));
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
