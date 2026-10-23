//! Op-merge properties (plan E3.2; C9.4 S4, S5, S6): the creator comes
//! only from the cert, strong removal, re-join, admission, invite caps,
//! equivocation and order independence.

use proptest::prelude::*;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry, InviteProof};
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions::{KICK_MEMBERS, MANAGE_COMMUNITY, VIEW_CHANNELS};

use super::{merge, merge_outcome};
use crate::fixture::{cert, meta, op, op_at};
use crate::invite_quota::MAX_USES_PER_INVITE;
use crate::op::VerifiedOp;
use crate::power::{class_of, OpClass};

fn pk(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn joined(code: Option<&str>) -> GovernanceEntry {
    GovernanceEntry::MemberJoined {
        member_record: "VLD0:m".into(),
        member_record_owner: "owner".into(),
        invite_proof: code.map(|c| InviteProof {
            code_hash: c.into(),
            sig: vec![0; 64],
        }),
    }
}

fn role(id: u8, position: u32) -> GovernanceEntry {
    GovernanceEntry::RoleDefinition {
        role_id: RoleId([id; 16]),
        name: format!("r{id}"),
        permissions: VIEW_CHANNELS | KICK_MEMBERS | MANAGE_COMMUNITY,
        position,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    }
}

fn assign(target: u8, id: u8) -> GovernanceEntry {
    GovernanceEntry::RoleAssignment {
        target: pk(target),
        role_id: RoleId([id; 16]),
    }
}

fn kick(target: u8) -> GovernanceEntry {
    GovernanceEntry::MemberRemoved {
        target: pk(target),
        reason: None,
    }
}

fn invite(max_uses: u32) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [7; 16],
        code_hash: "code".into(),
        max_uses,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    }
}

/// Creator 1; managers 2 (role 6) and 3 (role 5); member 4. Returns the
/// ops and the head everyone has seen.
fn community() -> Vec<VerifiedOp> {
    let c0 = op(&pk(1), 0, &[], 0, joined(None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(MAX_USES_PER_INVITE));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(Some("code")));
    let b0 = op(&pk(3), 0, &[&c1], 3, joined(Some("code")));
    let d0 = op(&pk(4), 0, &[&c1], 4, joined(Some("code")));
    let c2 = op(&pk(1), 2, &[&c1, &a0, &b0, &d0], 5, role(6, 6));
    let c3 = op(&pk(1), 3, &[&c2], 6, role(5, 5));
    let c4 = op(&pk(1), 4, &[&c3], 7, assign(2, 6));
    let c5 = op(&pk(1), 5, &[&c4], 8, assign(3, 5));
    vec![c0, c1, a0, b0, d0, c2, c3, c4, c5]
}

fn head(ops: &[VerifiedOp]) -> &VerifiedOp {
    ops.last().expect("ops")
}

fn find(ops: &[VerifiedOp], author: u8, seq: u64) -> &VerifiedOp {
    ops.iter()
        .find(|o| o.author() == &pk(author) && o.op().seq == seq)
        .expect("op")
}

#[test]
fn every_variant_has_a_power_row() {
    // The match in `class_of` is exhaustive, so this compiles only with a
    // row per variant; the rows below pin the ones resolution relies on.
    assert_eq!(class_of(&kick(2)), OpClass::Power);
    assert_eq!(class_of(&assign(2, 6)), OpClass::Power);
    assert_eq!(class_of(&joined(None)), OpClass::NonPower);
    assert_eq!(class_of(&GovernanceEntry::MemberLeft {}), OpClass::NonPower);
    assert_eq!(class_of(&meta("x")), OpClass::NonPower);
}

#[test]
fn members_join_by_invite() {
    let ops = community();
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    for m in 1..=4 {
        assert!(state.members.contains_key(&pk(m)), "member {m}");
    }
}

#[test]
fn a_join_without_a_live_invite_is_refused() {
    let mut ops = community();
    let stray = op(&pk(9), 0, &[head(&ops)], 20, joined(Some("other")));
    let early = op(&pk(8), 0, &[find(&ops, 1, 0)], 21, joined(Some("code")));
    ops.extend([stray, early]);
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!state.members.contains_key(&pk(9)), "unknown commitment");
    assert!(
        !state.members.contains_key(&pk(8)),
        "invite not in its causal past"
    );
}

/// A join dated before its own deps is refused (register SEC-3): here
/// before an `InviteCreated` that expires before the deps' latest time, so
/// a backdated join can't slip under the expiry.
#[test]
fn a_join_backdated_before_its_deps_is_refused() {
    let c0 = op(&pk(1), 0, &[], 0, joined(None));
    let expiring = GovernanceEntry::InviteCreated {
        invite_id: [7; 16],
        code_hash: "code".into(),
        max_uses: MAX_USES_PER_INVITE,
        expires_at: Some(100),
        secrets_record_key: "VLD0:s".into(),
    };
    let c1 = op(&pk(1), 1, &[&c0], 1, expiring);
    let late = op(&pk(1), 2, &[&c1], 2, meta("after expiry"));
    let late = VerifiedOp::assume_verified(
        rekindle_types::governance::GovernanceOp {
            issued_at: 500,
            ..late.op().clone()
        },
        late.hash(),
        late.lamport(),
    );
    let honest = op(&pk(2), 0, &[&c1], 3, joined(Some("code")));
    let backdated = op(&pk(3), 0, &[&late], 4, joined(Some("code")));
    let backdated = VerifiedOp::assume_verified(
        rekindle_types::governance::GovernanceOp {
            issued_at: 50,
            ..backdated.op().clone()
        },
        backdated.hash(),
        backdated.lamport(),
    );
    let state = merge(
        &cert(&pk(1), AdmissionMode::Open),
        &[c0, c1, late, honest, backdated],
    );
    assert!(
        state.members.contains_key(&pk(2)),
        "a join dated after its deps"
    );
    assert!(
        !state.members.contains_key(&pk(3)),
        "a join dated before its deps' latest time"
    );
}

#[test]
fn invite_max_uses_caps_joins() {
    let c0 = op(&pk(1), 0, &[], 0, joined(None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(1));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(Some("code")));
    let b0 = op(&pk(3), 0, &[&c1], 3, joined(Some("code")));
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &[c0, c1, a0, b0]);
    let joined_count = [2, 3]
        .iter()
        .filter(|m| state.members.contains_key(&pk(**m)))
        .count();
    assert_eq!(joined_count, 1);
}

#[test]
fn approval_required_holds_joiners_pending() {
    let c0 = op(&pk(1), 0, &[], 0, joined(None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(MAX_USES_PER_INVITE));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(Some("code")));
    let b0 = op(&pk(3), 0, &[&c1], 3, joined(Some("code")));
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
    let state = merge(&cert, &[c0, c1, a0, b0, a1, approve, reject]);
    assert!(state.members.contains_key(&pk(2)), "approved");
    assert!(!state.members.contains_key(&pk(3)), "rejected");
    assert!(!state.pending_members.contains_key(&pk(3)));
}

#[test]
fn removal_invalidates_the_removed_managers_concurrent_actions() {
    let mut ops = community();
    // Manager 2 (rank 6) removes manager 3 (rank 5) while 3, unaware,
    // removes member 4: 3's concurrent kick is invalidated.
    let k2 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 30, kick(3));
    let k3 = op(&pk(3), 1, &[head(&ops), find(&ops, 3, 0)], 31, kick(4));
    ops.extend([k2, k3]);
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!state.members.contains_key(&pk(3)));
    assert!(
        state.members.contains_key(&pk(4)),
        "the removed manager's kick is void"
    );
}

#[test]
fn rejoin_after_removal_is_allowed_and_concurrent_acts_stay_void() {
    let mut ops = community();
    let k = op(&pk(1), 6, &[head(&ops)], 40, kick(4));
    let concurrent = op(
        &pk(4),
        1,
        &[head(&ops), find(&ops, 4, 0)],
        41,
        joined(Some("code")),
    );
    let rejoin = op(&pk(4), 2, &[&k, &concurrent], 42, joined(Some("code")));
    ops.extend([k.clone(), concurrent.clone()]);
    let removed = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!removed.members.contains_key(&pk(4)));
    ops.push(rejoin);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(outcome.state.members.contains_key(&pk(4)), "re-joined");
    assert!(outcome.refused.contains(&concurrent.hash()));
}

#[test]
fn strong_removal_chains_transitively() {
    // Under the hierarchy (target rank < writer rank) a mutual kick is
    // refused on both sides, so the case that matters is the chain
    // (decision (a), p2panda-auth): A (2, rank 6) removes B (3, rank 5)
    // concurrently with B removing C (5, rank 4), while C, unaware, kicks
    // member 4. B's removal of C is invalidated, so C stays a manager and
    // C's kick, no longer under a valid removal, applies.
    let mut ops = community();
    let c_join = op(&pk(5), 0, &[head(&ops)], 50, joined(Some("code")));
    let c_role = op(&pk(1), 6, &[&c_join], 51, role(4, 4));
    let c_assign = op(&pk(1), 7, &[&c_role], 52, assign(5, 4));
    ops.extend([c_join, c_role, c_assign]);
    let seen = head(&ops).clone();
    let a = op(&pk(2), 1, &[&seen, find(&ops, 2, 0)], 53, kick(3));
    let b = op(&pk(3), 1, &[&seen, find(&ops, 3, 0)], 54, kick(5));
    let c = op(&pk(5), 1, &[&seen, find(&ops, 5, 0)], 55, kick(4));
    ops.extend([a.clone(), b.clone(), c.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!outcome.state.members.contains_key(&pk(3)), "A removed B");
    assert!(
        outcome.refused.contains(&b.hash()),
        "B's removal of C is void"
    );
    assert!(outcome.state.members.contains_key(&pk(5)), "C stays");
    assert!(outcome.applied.contains(&c.hash()), "C's own kick stands");
    assert!(!outcome.state.members.contains_key(&pk(4)));
}

#[test]
fn an_invite_without_a_use_cap_is_refused() {
    let c0 = op(&pk(1), 0, &[], 0, joined(None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(0));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(Some("code")));
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &[c0, c1.clone(), a0]);
    assert!(
        outcome.refused.contains(&c1.hash()),
        "max_uses 0 is not unlimited"
    );
    assert!(outcome.state.invites.is_empty());
    assert!(
        !outcome.state.members.contains_key(&pk(2)),
        "no live invite"
    );
}

#[test]
fn equivocation_is_detected_and_the_fork_dropped() {
    let mut ops = community();
    let f1 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 60, meta("one"));
    let f2 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 61, meta("two"));
    ops.extend([f1.clone(), f2.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert_eq!(outcome.equivocators, vec![pk(2)]);
    assert!(!outcome.applied.contains(&f1.hash()) && !outcome.applied.contains(&f2.hash()));
}

#[test]
fn an_op_missing_a_dep_waits() {
    let ops = community();
    let orphan_dep = op(&pk(5), 0, &[], 70, meta("never delivered"));
    let waiting = op(
        &pk(2),
        1,
        &[&orphan_dep, find(&ops, 2, 0)],
        71,
        meta("waits"),
    );
    let mut set = ops;
    set.push(waiting.clone());
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &set);
    assert_eq!(outcome.pending, vec![waiting.hash()]);
}

proptest! {
    /// S4: an op at any lamport, 0 included, never becomes the creator, and
    /// a forger's power op is refused.
    #[test]
    fn no_op_at_any_lamport_becomes_the_creator(lamport in any::<u64>()) {
        let ops = community();
        let forged = op_at(&pk(9), 0, &[], 80, lamport, role(9, 100));
        let mut set = vec![forged.clone()];
        set.extend(ops);
        let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &set);
        prop_assert_eq!(outcome.state.creator, Some(pk(1)));
        prop_assert!(!outcome.applied.contains(&forged.hash()));
    }

    /// S5/S6: two nodes holding one op set, received in any order, reach
    /// equal state.
    #[test]
    fn one_op_set_one_state(seed in any::<u64>()) {
        let mut ops = community();
        let k2 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 30, kick(3));
        let k3 = op(&pk(3), 1, &[head(&ops), find(&ops, 3, 0)], 31, kick(4));
        let m = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 32, meta("m"));
        ops.extend([k2, k3, m]);
        let mut shuffled = ops.clone();
        let len = shuffled.len();
        let mut x = seed | 1;
        for i in (1..len).rev() {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            shuffled.swap(i, usize::try_from(x % (i as u64 + 1)).unwrap_or(0));
        }
        let cert = cert(&pk(1), AdmissionMode::Open);
        prop_assert_eq!(merge(&cert, &ops), merge(&cert, &shuffled));
    }
}
