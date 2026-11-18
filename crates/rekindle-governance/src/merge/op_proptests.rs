//! Op-merge properties (plan E3.2; C9.4 S4, S5, S6): the creator comes
//! only from the cert, strong removal, re-join, admission, invite caps,
//! equivocation and order independence.

use proptest::prelude::*;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry, InviteProof};
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions::{CREATE_INVITES, KICK_MEMBERS, MANAGE_COMMUNITY, VIEW_CHANNELS};

use super::{merge, merge_outcome, MergeOutcome, QuarantineReason};
use crate::fixture::{cert, code_key, meta, op, op_at};
use crate::invite_quota::MAX_USES_PER_INVITE;
use crate::op::VerifiedOp;
use crate::power::{class_of, OpClass};

pub(super) fn pk(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

pub(super) fn joined(by: u8, code: Option<&str>) -> GovernanceEntry {
    GovernanceEntry::MemberJoined {
        member_record: format!("VLD0:m{by}"),
        member_record_owner: "owner".into(),
        invite_proof: code.map(|c| InviteProof {
            code_hash: code_key(c),
            sig: vec![0; 64],
        }),
        owner_proof: vec![0; 64],
    }
}

pub(super) fn role(id: u8, position: u32) -> GovernanceEntry {
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

pub(super) fn assign(target: u8, id: u8) -> GovernanceEntry {
    GovernanceEntry::RoleAssignment {
        target: pk(target),
        role_id: RoleId([id; 16]),
    }
}

/// A durable kick of `target`, `frontier` of its ops in the kick's past.
pub(super) fn kick(target: u8, frontier: u64) -> GovernanceEntry {
    GovernanceEntry::MemberRemoved {
        target: pk(target),
        reason: None,
        heads: Vec::new(),
        target_seq_frontier: frontier,
    }
}

pub(super) fn invite(max_uses: u32) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [7; 16],
        code_hash: code_key("code"),
        max_uses,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    }
}

/// Creator 1; managers 2 (role 6) and 3 (role 5); member 4. Returns the
/// ops and the head everyone has seen.
pub(super) fn community() -> Vec<VerifiedOp> {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(MAX_USES_PER_INVITE));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
    let b0 = op(&pk(3), 0, &[&c1], 3, joined(3, Some("code")));
    let d0 = op(&pk(4), 0, &[&c1], 4, joined(4, Some("code")));
    let c2 = op(&pk(1), 2, &[&c1, &a0, &b0, &d0], 5, role(6, 6));
    let c3 = op(&pk(1), 3, &[&c2], 6, role(5, 5));
    let c4 = op(&pk(1), 4, &[&c3], 7, assign(2, 6));
    let c5 = op(&pk(1), 5, &[&c4], 8, assign(3, 5));
    vec![c0, c1, a0, b0, d0, c2, c3, c4, c5]
}

pub(super) fn head(ops: &[VerifiedOp]) -> &VerifiedOp {
    ops.last().expect("ops")
}

/// Whether `masked` is quarantined after its author's removal `cause`.
pub(super) fn quarantined_by(
    outcome: &MergeOutcome,
    masked: &VerifiedOp,
    cause: &VerifiedOp,
) -> bool {
    outcome.quarantined.contains(&(
        masked.hash(),
        QuarantineReason::AfterAuthorRemoval {
            cause: cause.hash(),
        },
    ))
}

pub(super) fn find(ops: &[VerifiedOp], author: u8, seq: u64) -> &VerifiedOp {
    ops.iter()
        .find(|o| o.author() == &pk(author) && o.op().seq == seq)
        .expect("op")
}

#[test]
fn every_variant_has_a_power_row() {
    // The match in `class_of` is exhaustive, so this compiles only with a
    // row per variant; the rows below pin the ones resolution relies on.
    assert_eq!(class_of(&kick(2, 1)), OpClass::Power);
    assert_eq!(class_of(&assign(2, 6)), OpClass::Power);
    assert_eq!(class_of(&joined(1, None)), OpClass::NonPower);
    assert_eq!(
        class_of(&GovernanceEntry::MemberLeft { heads: Vec::new() }),
        OpClass::NonPower
    );
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
    let stray = op(&pk(9), 0, &[head(&ops)], 20, joined(9, Some("other")));
    let early = op(&pk(8), 0, &[find(&ops, 1, 0)], 21, joined(8, Some("code")));
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
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let expiring = GovernanceEntry::InviteCreated {
        invite_id: [7; 16],
        code_hash: code_key("code"),
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
    let honest = op(&pk(2), 0, &[&c1], 3, joined(2, Some("code")));
    let backdated = op(&pk(3), 0, &[&late], 4, joined(3, Some("code")));
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

/// Who may revoke an invite (register FX1 SEC-3): its creator, or a
/// `MANAGE_COMMUNITY` holder. A `CREATE_INVITES`-only member's revocation
/// of its own invite is admitted, and of someone else's is refused.
#[test]
fn an_invite_creator_may_revoke_only_its_own_invite() {
    let ops = community();
    let inviter_role = GovernanceEntry::RoleDefinition {
        role_id: RoleId([8; 16]),
        name: "inviter".into(),
        permissions: VIEW_CHANNELS | CREATE_INVITES,
        position: 1,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    };
    let c6 = op(&pk(1), 6, &[head(&ops)], 9, inviter_role);
    let c7 = op(&pk(1), 7, &[&c6], 10, assign(4, 8));
    let own = GovernanceEntry::InviteCreated {
        invite_id: [9; 16],
        code_hash: code_key("own"),
        max_uses: MAX_USES_PER_INVITE,
        expires_at: None,
        secrets_record_key: "VLD0:o".into(),
    };
    let d1 = op(&pk(4), 1, &[&c7], 11, own);
    let d2 = op(
        &pk(4),
        2,
        &[&d1],
        12,
        GovernanceEntry::InviteRevoked { invite_id: [9; 16] },
    );
    let d3 = op(
        &pk(4),
        3,
        &[&d2],
        13,
        GovernanceEntry::InviteRevoked { invite_id: [7; 16] },
    );
    let mut all = ops.clone();
    all.extend([c6, c7, d1.clone()]);
    let before = merge(&cert(&pk(1), AdmissionMode::Open), &all);
    assert!(before.invites.contains_key(&[9; 16]), "own invite created");
    all.extend([d2, d3]);
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &all);
    assert!(
        !state.invites.contains_key(&[9; 16]),
        "the creator's revocation of its own invite is admitted"
    );
    assert!(
        state.invites.contains_key(&[7; 16]),
        "a CREATE_INVITES-only revocation of another's invite is refused"
    );
}

#[test]
fn invite_max_uses_caps_joins() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(1));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
    let b0 = op(&pk(3), 0, &[&c1], 3, joined(3, Some("code")));
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &[c0, c1, a0, b0]);
    let joined_count = [2, 3]
        .iter()
        .filter(|m| state.members.contains_key(&pk(**m)))
        .count();
    assert_eq!(joined_count, 1);
}

/// The merge writes the joins `invite_gate` counted into each invite's
/// `uses` (plan E3.2 S7c.3), whatever order the ops arrive in, so a
/// checkpoint's `stateRoot` commits to them; a join past the cap is not
/// counted.
#[test]
fn invite_uses_are_in_the_state() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(2));
    let mut ops = vec![c0, c1];
    let uses =
        |ops: &[VerifiedOp]| merge(&cert(&pk(1), AdmissionMode::Open), ops).invites[&[7; 16]].uses;
    assert_eq!(uses(&ops), 0, "no join, no use");
    let joins: Vec<VerifiedOp> = (2..=4)
        .map(|m| op(&pk(m), 0, &[&ops[1]], u64::from(m), joined(m, Some("code"))))
        .collect();
    ops.extend(joins);
    assert_eq!(uses(&ops), 2, "two joins counted, the third past the cap");
    ops.reverse();
    assert_eq!(uses(&ops), 2);
}

#[test]
fn removal_invalidates_the_removed_managers_concurrent_actions() {
    let mut ops = community();
    // Manager 2 (rank 6) removes manager 3 (rank 5) while 3, unaware,
    // removes member 4: 3's concurrent kick is invalidated.
    let k2 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 30, kick(3, 1));
    let k3 = op(&pk(3), 1, &[head(&ops), find(&ops, 3, 0)], 31, kick(4, 1));
    ops.extend([k2.clone(), k3.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!outcome.state.members.contains_key(&pk(3)));
    assert!(
        outcome.state.members.contains_key(&pk(4)),
        "the removed manager's kick is void"
    );
    assert!(
        quarantined_by(&outcome, &k3, &k2),
        "masked by k2's frontier"
    );
}

#[test]
fn a_removal_names_its_author_as_the_remover() {
    let mut ops = community();
    let k = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 30, kick(4, 1));
    let ban = op(
        &pk(1),
        6,
        &[head(&ops)],
        31,
        GovernanceEntry::BanEntry {
            target: pk(3),
            reason: Some("spam".into()),
            target_seq_frontier: 1,
        },
    );
    ops.extend([k, ban]);
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert_eq!(state.removed_by.get(&pk(4)), Some(&pk(2)), "kicked by 2");
    assert_eq!(state.removed_by.get(&pk(3)), Some(&pk(1)), "banned by 1");
    assert!(!state.removed_by.contains_key(&pk(2)));
    // A/B R5-2: the removed member reads the reason the op gave.
    assert_eq!(
        state.removal_reasons.get(&pk(3)).map(String::as_str),
        Some("spam")
    );
    assert!(
        !state.removal_reasons.contains_key(&pk(4)),
        "the kick gave none"
    );
}

#[test]
fn rejoin_after_removal_is_allowed_and_concurrent_acts_stay_void() {
    let mut ops = community();
    let k = op(&pk(1), 6, &[head(&ops)], 40, kick(4, 1));
    let concurrent = op(
        &pk(4),
        1,
        &[head(&ops), find(&ops, 4, 0)],
        41,
        joined(4, Some("code")),
    );
    let rejoin = op(&pk(4), 2, &[&k, &concurrent], 42, joined(4, Some("code")));
    ops.extend([k.clone(), concurrent.clone()]);
    let removed = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!removed.members.contains_key(&pk(4)));
    ops.push(rejoin);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(outcome.state.members.contains_key(&pk(4)), "re-joined");
    // The concurrent join is masked by the kick's seq frontier.
    assert!(quarantined_by(&outcome, &concurrent, &k));
    assert!(!outcome.refused.contains(&concurrent.hash()));
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
    let c_join = op(&pk(5), 0, &[head(&ops)], 50, joined(5, Some("code")));
    let c_role = op(&pk(1), 6, &[&c_join], 51, role(4, 4));
    let c_assign = op(&pk(1), 7, &[&c_role], 52, assign(5, 4));
    ops.extend([c_join, c_role, c_assign]);
    let seen = head(&ops).clone();
    let a = op(&pk(2), 1, &[&seen, find(&ops, 2, 0)], 53, kick(3, 1));
    let b = op(&pk(3), 1, &[&seen, find(&ops, 3, 0)], 54, kick(5, 1));
    let c = op(&pk(5), 1, &[&seen, find(&ops, 5, 0)], 55, kick(4, 1));
    ops.extend([a.clone(), b.clone(), c.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!outcome.state.members.contains_key(&pk(3)), "A removed B");
    assert!(
        quarantined_by(&outcome, &b, &a),
        "B's removal of C is masked by A's frontier"
    );
    assert!(!outcome.refused.contains(&b.hash()));
    assert!(outcome.state.members.contains_key(&pk(5)), "C stays");
    assert!(outcome.applied.contains(&c.hash()), "C's own kick stands");
    assert!(!outcome.state.members.contains_key(&pk(4)));
}

#[test]
fn an_invite_without_a_use_cap_is_refused() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(0));
    let a0 = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
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

/// Two members whose joins name one member record: only the first in the
/// op set's stamp order holds it, the other's join is refused, alike in
/// either arrival order; a join naming the creator's record is refused.
#[test]
fn two_joins_naming_one_member_record_admit_only_the_first() {
    let claim = |by: u8, record: &str| GovernanceEntry::MemberJoined {
        member_record: record.into(),
        member_record_owner: format!("owner{by}"),
        invite_proof: Some(InviteProof {
            code_hash: code_key("code"),
            sig: vec![0; 64],
        }),
        owner_proof: vec![0; 64],
    };
    let base = community();
    let x = op(&pk(5), 0, &[head(&base)], 60, claim(5, "VLD0:shared"));
    let y = op(&pk(6), 0, &[head(&base)], 61, claim(6, "VLD0:shared"));
    let z = op(
        &pk(7),
        0,
        &[head(&base)],
        62,
        claim(7, "VLD0:creator-record"),
    );
    let (first, second) = if x.stamp() < y.stamp() {
        (&x, &y)
    } else {
        (&y, &x)
    };
    for order in [[&x, &y, &z], [&z, &y, &x]] {
        let mut ops = base.clone();
        ops.extend(order.iter().map(|o| (*o).clone()));
        let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
        let members = &outcome.state.members;
        assert!(members.contains_key(first.author()), "the first holds it");
        assert!(
            !members.contains_key(second.author()),
            "the second is refused"
        );
        assert!(outcome.refused.contains(&second.hash()));
        assert!(outcome.applied.contains(&first.hash()));
        assert!(
            !members.contains_key(&pk(7)),
            "the creator's record is held"
        );
        assert!(outcome.refused.contains(&z.hash()));
    }
}

/// A `MemberRecordMoved` onto a record another member holds (or the
/// creator's) is refused like a second join on it; the mover keeps its own.
#[test]
fn a_move_onto_a_held_member_record_is_refused() {
    let claim = |record: &str| GovernanceEntry::MemberJoined {
        member_record: record.into(),
        member_record_owner: "owner".into(),
        invite_proof: Some(InviteProof {
            code_hash: code_key("code"),
            sig: vec![0; 64],
        }),
        owner_proof: vec![0; 64],
    };
    let moved = |record: &str| GovernanceEntry::MemberRecordMoved {
        member_record: record.into(),
        member_record_owner: "owner".into(),
        owner_proof: vec![0; 64],
    };
    let mut ops = community();
    let x = op(&pk(5), 0, &[head(&ops)], 60, claim("VLD0:mx"));
    let y = op(&pk(6), 0, &[head(&ops)], 61, claim("VLD0:my"));
    ops.extend([x.clone(), y.clone()]);
    let onto_x = op(&pk(6), 1, &[&x, &y], 62, moved("VLD0:mx"));
    let onto_creator = op(&pk(5), 1, &[&x, &y], 63, moved("VLD0:creator-record"));
    ops.extend([onto_x.clone(), onto_creator.clone()]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(outcome.refused.contains(&onto_x.hash()));
    assert!(outcome.refused.contains(&onto_creator.hash()));
    let record = |p: u8| outcome.state.members[&pk(p)].member_record.clone();
    assert_eq!((record(5), record(6)), ("VLD0:mx".into(), "VLD0:my".into()));
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

/// R4-5 pins the fork scan's semantics: a fork whose two ops are apart in
/// stamp order is found, and one op held at two lamports (one hash) is no
/// fork.
#[test]
fn a_fork_apart_in_stamp_order_is_found_and_one_hash_twice_is_not() {
    let mut ops = community();
    let f1 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 60, meta("one"));
    let x = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 61, meta("x"));
    let f2 = op(&pk(2), 1, &[&x, find(&ops, 2, 0)], 62, meta("two"));
    let once = op(&pk(3), 1, &[head(&ops), find(&ops, 3, 0)], 63, meta("y"));
    let again = op(&pk(3), 1, &[&x, find(&ops, 3, 0)], 63, meta("y"));
    assert!(f1.lamport() < f2.lamport() && once.hash() == again.hash());
    ops.extend([f1, x, f2, once, again]);
    let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert_eq!(outcome.equivocators, vec![pk(2)]);
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
        let k2 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 30, kick(3, 1));
        let k3 = op(&pk(3), 1, &[head(&ops), find(&ops, 3, 0)], 31, kick(4, 1));
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

#[path = "departure_tests.rs"]
mod departure_tests;
#[path = "time_of_op_tests.rs"]
mod time_of_op_tests;
