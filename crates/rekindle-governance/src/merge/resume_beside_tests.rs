//! A non-power op beside the cut (plan E3.2 S7c.8 (3)): a join signed
//! concurrently with a checkpoint's cut cites the old base, not the cut's
//! heads. `merge_from` answers for it from the snapshot exactly when the
//! full merge would place and judge it the same way, and `NotOnTop`
//! otherwise; every `Ok` is checked against `merge_outcome` by `resume`.

use rekindle_types::governance::{AdmissionMode, GenesisCert, GovernanceEntry, OpHash};

use super::op_proptests::{community, find, head, joined, kick, pk};
use super::resume_tests::resume;
use super::NotOnTop;
use crate::fixture::{cert, meta, op};
use crate::op::VerifiedOp;

fn base() -> (GenesisCert, Vec<VerifiedOp>) {
    (cert(&pk(1), AdmissionMode::Open), community())
}

/// `o` re-dated to `issued_at` (above the cut's mainline, as a join signed
/// after the joins in the cut is).
fn dated(o: &VerifiedOp, issued_at: u64) -> VerifiedOp {
    let mut body = o.op().clone();
    body.issued_at = issued_at;
    VerifiedOp::assume_verified(body, o.hash(), o.lamport())
}

/// The first op `make(tag)` builds, for tags from `from` on, whose hash
/// sorts below `than`: a stamp before a cut op of the same lamport.
fn below(than: &VerifiedOp, from: u64, make: impl Fn(u64) -> VerifiedOp) -> VerifiedOp {
    (from..from + 512)
        .map(make)
        .find(|o| o.stamp() < than.stamp())
        .expect("a hash below")
}

fn invite(id: u8, code: &str, max_uses: u32) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [id; 16],
        code_hash: crate::fixture::code_key(code),
        max_uses,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    }
}

/// Joins 6 and 7 on the creator's last op are the cut's heads; join 8 on
/// the same op races the checkpoint (its deps a strict subset of the
/// heads), and join 9 cites the whole cut.
#[test]
fn a_join_racing_the_cut_resumes_and_equals_the_full_merge() {
    let (cert, mut ops) = base();
    let c5 = head(&ops).clone();
    let j6 = op(&pk(6), 0, &[&c5], 60, joined(6, Some("code")));
    let j7 = op(&pk(7), 0, &[&c5], 61, joined(7, Some("code")));
    let j8 = dated(&op(&pk(8), 0, &[&c5], 62, joined(8, Some("code"))), 100);
    let j9 = op(&pk(9), 0, &[&j6, &j7], 63, joined(9, Some("code")));
    let raced = j8.hash();
    ops.extend([j6.clone(), j7.clone(), j8, j9]);
    let resumed = resume(&cert, &ops, &[&j6, &j7]).expect("on top");
    assert!(resumed.applied.contains(&raced), "the racing join applies");
}

/// A post-cut op that follows the racing join and the cut is on top too.
#[test]
fn an_op_after_the_racing_join_resumes() {
    let (cert, mut ops) = base();
    let c5 = head(&ops).clone();
    let j6 = op(&pk(6), 0, &[&c5], 60, joined(6, Some("code")));
    let j7 = op(&pk(7), 0, &[&c5], 61, joined(7, Some("code")));
    let j8 = dated(&op(&pk(8), 0, &[&c5], 62, joined(8, Some("code"))), 100);
    let m8 = op(
        &pk(8),
        1,
        &[&j8, &j6, &j7],
        64,
        GovernanceEntry::MemberLeft { heads: Vec::new() },
    );
    let left = m8.hash();
    ops.extend([j6.clone(), j7.clone(), j8, m8]);
    let resumed = resume(&cert, &ops, &[&j6, &j7]).expect("on top");
    assert!(resumed.applied.contains(&left));
}

/// Manager 3's role is assigned by the cut's last power op (`c5`); an op
/// of 3 beside the cut that lacks it is judged before it by the full
/// merge (its anchor is `c4`), so the snapshot cannot answer for it.
#[test]
fn a_beside_op_missing_a_cut_power_op_is_not_on_top() {
    let (cert, mut ops) = base();
    let c5 = head(&ops).clone();
    let j6 = op(&pk(6), 0, &[&c5], 60, joined(6, Some("code")));
    let m3 = op(
        &pk(3),
        1,
        &[find(&ops, 3, 0), find(&ops, 1, 4)],
        61,
        meta("renamed"),
    );
    let m3 = dated(&m3, 100);
    let hash = m3.hash();
    ops.extend([j6.clone(), m3]);
    assert_eq!(
        resume(&cert, &ops, &[&j6]).err(),
        Some(NotOnTop::BesideTheCut(hash))
    );
}

/// An invite with one use: the cut's join 6 used it, and join 8 beside the
/// cut sorts before join 6 in the full merge's stamp order, so the full
/// merge gives the use to 8 and refuses 6, a verdict the snapshot fixed.
#[test]
fn a_beside_join_on_a_spent_invite_is_not_on_top() {
    let (cert, mut ops) = base();
    let c6 = op(&pk(1), 6, &[head(&ops)], 59, invite(8, "one", 1));
    let j6 = op(&pk(6), 0, &[&c6], 60, joined(6, Some("one")));
    let j8 = below(&j6, 70, |t| {
        op(&pk(8), 0, &[&c6], t, joined(8, Some("one")))
    });
    let j8 = dated(&j8, 100);
    let hash = j8.hash();
    ops.extend([c6, j6.clone(), j8]);
    assert_eq!(
        resume(&cert, &ops, &[&j6]).err(),
        Some(NotOnTop::BesideTheCut(hash))
    );
}

/// The cut revokes the invite, but not in the past of join 8 beside it:
/// the full merge admits 8 (the revocation is concurrent), the snapshot
/// only knows the invite is revoked.
#[test]
fn a_beside_join_concurrent_with_a_cut_revocation_is_not_on_top() {
    let (cert, mut ops) = base();
    let c5 = head(&ops).clone();
    let c6 = op(
        &pk(1),
        6,
        &[&c5],
        59,
        GovernanceEntry::InviteRevoked { invite_id: [7; 16] },
    );
    let j8 = dated(&op(&pk(8), 0, &[&c5], 62, joined(8, Some("code"))), 100);
    let hash = j8.hash();
    ops.extend([c6.clone(), j8]);
    assert_eq!(
        resume(&cert, &ops, &[&c6]).err(),
        Some(NotOnTop::BesideTheCut(hash))
    );
}

/// Join 8 beside the cut claims the member record join 6 holds in the cut
/// and sorts before it: the full merge gives the record to 8.
#[test]
fn a_beside_join_claiming_a_cut_record_is_not_on_top() {
    let (cert, mut ops) = base();
    let c5 = head(&ops).clone();
    let j6 = op(&pk(6), 0, &[&c5], 60, joined(6, Some("code")));
    let j8 = below(&j6, 70, |t| {
        op(&pk(8), 0, &[&c5], t, joined(6, Some("code")))
    });
    let j8 = dated(&j8, 100);
    let hash: OpHash = j8.hash();
    ops.extend([j6.clone(), j8]);
    assert_eq!(
        resume(&cert, &ops, &[&j6]).err(),
        Some(NotOnTop::BesideTheCut(hash))
    );
}

/// The cut creates invite "two", but join 8 beside the cut on it cites
/// only the op before: the full merge finds no invite in its past and
/// refuses it, the snapshot only knows the invite exists.
#[test]
fn a_beside_join_without_its_invite_in_its_past_is_not_on_top() {
    let (cert, mut ops) = base();
    let c5 = head(&ops).clone();
    let c6 = op(&pk(1), 6, &[&c5], 59, invite(8, "two", 5));
    let j8 = dated(&op(&pk(8), 0, &[&c5], 62, joined(8, Some("two"))), 100);
    let hash = j8.hash();
    ops.extend([c6.clone(), j8]);
    assert_eq!(
        resume(&cert, &ops, &[&c6]).err(),
        Some(NotOnTop::BesideTheCut(hash))
    );
}

/// A power op beside the cut could reorder the cut's power phase.
#[test]
fn merge_from_answers_not_on_top_for_a_concurrent_power_op() {
    let (cert, mut ops) = base();
    let m = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 40, meta("x"));
    ops.push(m);
    // Manager 2 kicks 3 without having seen 4's latest op, the cut head.
    let k = op(
        &pk(2),
        1,
        &[&ops[ops.len() - 2], find(&ops, 2, 0)],
        41,
        kick(3, 1),
    );
    let hash = k.hash();
    ops.push(k);
    let heads = [&ops[ops.len() - 2]];
    assert_eq!(
        resume(&cert, &ops, &heads).err(),
        Some(NotOnTop::BesideTheCut(hash))
    );
}

#[test]
fn merge_from_answers_not_on_top_when_a_kick_frontier_is_below_the_cut() {
    let (cert, mut ops) = base();
    let m = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 40, meta("x"));
    ops.push(m);
    // 2 kicks 4 having seen only 4's seq 0: its frontier, 1, is below 4's
    // last seq in the cut, 1.
    let k = op(
        &pk(2),
        1,
        &[&ops[ops.len() - 2], find(&ops, 2, 0)],
        41,
        kick(4, 1),
    );
    let hash = k.hash();
    ops.push(k);
    let heads = [&ops[ops.len() - 2]];
    assert_eq!(
        resume(&cert, &ops, &heads).err(),
        Some(NotOnTop::BesideTheCut(hash))
    );
}
