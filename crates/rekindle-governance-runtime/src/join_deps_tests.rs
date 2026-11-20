//! Shallow joins (plan E3.2 S7c.0): a join cites the creator's last
//! genesis op and the inviter's latest op, never the previous joiner, so
//! the DAG's depth stops growing with the number of joins; a fold join
//! cites at most 64 deps; a re-join cites its own chain and its removal.

use super::*;
use crate::op_log::{causal_past, frontier_refs, Deps};
use crate::origin::GENESIS_LAST_SEQ;
use crate::test_member::Member;
use crate::test_world::{invite, World};
use rekindle_codec::governance_op::{decode_op, encode_op};
use rekindle_governance::merge;
use rekindle_types::governance::GovernanceEntry;

fn dep_hashes(op: &VerifiedOp) -> BTreeSet<OpHash> {
    op.op().deps.iter().map(|d| d.op_hash).collect()
}

#[test]
fn a_join_depends_on_the_creators_seq_four_op_and_the_inviter_not_the_previous_joiner() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let base = world.op_at(&creator, GENESIS_LAST_SEQ);
    let invite_op = world.op_at(&creator, GENESIS_LAST_SEQ + 1);
    for seed in 10..20 {
        let joined = world.join(&Member::new(seed), &creator, "code");
        assert_eq!(dep_hashes(&joined), BTreeSet::from([base, invite_op]));
        assert!(world
            .state
            .members
            .contains_key(&Member::new(seed).author()));
    }
    // An invite of a member: the join cites that member's latest op.
    let inviter = Member::new(10);
    let own_invite = world.author(&inviter, invite(2, "code2"), Deps::Frontier);
    let joined = world.join(&Member::new(30), &inviter, "code2");
    assert_eq!(
        dep_hashes(&joined),
        BTreeSet::from([base, own_invite.hash()])
    );
    assert!(world.state.members.contains_key(&Member::new(30).author()));
}

#[test]
fn seventy_back_to_back_joins_stay_reachable_from_one_frontier() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    for seed in 10..80 {
        let joined = world.join(&Member::new(seed), &creator, "code");
        assert!(joined.op().deps.len() <= GOVERNANCE_OP_MAX_DEPS);
        // The invite op is at lamport 6: a join at 7, and each fold (one
        // per 33 joins) one more; citing the frontier, it would reach 76.
        assert!(
            joined.lamport() <= 9,
            "join depth grew: {}",
            joined.lamport()
        );
    }
    let frontier = frontier_refs(&world.cert, &world.state, &world.set);
    assert!(frontier.len() <= MAX_FRONTIER_REFS);
    assert_eq!(
        causal_past(&world.set.ops, &frontier).len(),
        world.set.ops.len()
    );
    assert_eq!(world.state.members.len(), 71);
}

#[test]
fn a_fold_join_cites_at_most_sixty_four_deps() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    // 70 concurrent joins, each after only the genesis and the invite.
    let before = world.held.clone();
    let mut concurrent = Vec::new();
    for seed in 10..80 {
        let member = Member::new(seed);
        let deps = world.deps_of(&member, &creator);
        concurrent.push(world.build(&member, member.joined("code"), Deps::Join(deps)));
    }
    assert_eq!(world.held, before);
    for (op, encoded) in concurrent {
        world.hold(&op, encoded);
    }
    let fold = world.join(&Member::new(90), &creator, "code");
    assert_eq!(fold.op().deps.len(), GOVERNANCE_OP_MAX_DEPS);
    let encoded = encode_op(fold.op()).unwrap();
    assert_eq!(decode_op(&encoded).unwrap().op, *fold.op());
    // The 8 joins past the fold stay heads, in the published frontier.
    let frontier = frontier_refs(&world.cert, &world.state, &world.set);
    assert_eq!(frontier.len(), 9);
    assert_eq!(
        causal_past(&world.set.ops, &frontier).len(),
        world.set.ops.len()
    );
    assert_eq!(world.state.members.len(), 72);
}

#[test]
fn a_re_join_cites_its_own_previous_op_and_its_removal() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let (rejoiner, inviter) = (Member::new(10), Member::new(11));
    let joined = world.join(&rejoiner, &creator, "code");
    world.join(&inviter, &creator, "code");
    let other_invite = world.author(&inviter, invite(2, "code2"), Deps::Frontier);
    // The removal is outside the inviter's past.
    let removal = world.author(
        &creator,
        GovernanceEntry::MemberRemoved {
            target: rejoiner.author(),
            reason: None,
            heads: Vec::new(),
            target_seq_frontier: 0,
        },
        Deps::Frontier,
    );
    assert!(!world.state.members.contains_key(&rejoiner.author()));
    let rejoined = world.join(&rejoiner, &inviter, "code2");
    let base = world.op_at(&creator, GENESIS_LAST_SEQ);
    assert_eq!(
        dep_hashes(&rejoined),
        BTreeSet::from([base, other_invite.hash(), joined.hash(), removal.hash()])
    );
    assert_eq!(rejoined.op().prev, Some(joined.hash()));
    assert!(world.state.members.contains_key(&rejoiner.author()));
}

/// Two members, the second holding its own invite `code2`, and an admin
/// op by the creator outside that member's past.
fn with_member_invite(admin_op: impl FnOnce(&Member) -> GovernanceEntry) -> (World, Member) {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let inviter = Member::new(11);
    world.join(&inviter, &creator, "code");
    world.author(&inviter, invite(2, "code2"), Deps::Frontier);
    let body = admin_op(&inviter);
    // Built after the creator's invite only: outside the inviter's past.
    world.author(&creator, body, Deps::Join(vec![world.op_at(&creator, 5)]));
    (world, inviter)
}

#[test]
fn a_revocation_outside_the_inviters_past_refuses_a_concurrent_join() {
    // The join sees only the inviter's chain and the base (C5), so it is
    // concurrent with the revoke, and a join outside its invite's revoke's
    // past is refused (plan SU1, lead decision: fail-closed, accepted).
    let (mut world, inviter) =
        with_member_invite(|_| GovernanceEntry::InviteRevoked { invite_id: [2; 16] });
    let joiner = Member::new(20);
    world.join(&joiner, &inviter, "code2");
    assert!(!world.state.members.contains_key(&joiner.author()));
}

#[test]
fn a_join_concurrent_with_the_inviters_removal_is_still_filtered() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let inviter = Member::new(11);
    let inviter_joined = world.join(&inviter, &creator, "code");
    world.author(&inviter, invite(2, "code2"), Deps::Frontier);
    // The joiner signs before it sees the removal.
    let joiner = Member::new(20);
    let deps = world.deps_of(&joiner, &inviter);
    let (joined, encoded) = world.build(&joiner, joiner.joined("code2"), Deps::Join(deps));
    // The removal follows the inviter's join, not its invite.
    let removal = GovernanceEntry::MemberRemoved {
        target: inviter.author(),
        reason: None,
        heads: Vec::new(),
        target_seq_frontier: 0,
    };
    let base = vec![world.op_at(&creator, 5), inviter_joined.hash()];
    world.author(&creator, removal, Deps::Join(base));
    world.hold(&joined, encoded);
    assert!(!world.state.members.contains_key(&inviter.author()));
    assert!(!world.state.members.contains_key(&joiner.author()));
}

/// A community whose member 10 created invite `code2`: the creator, its
/// invite, member 10's join and its invite op.
fn with_member_inviter(creator: &Member) -> (World, Member, VerifiedOp) {
    let mut world = World::new(creator);
    let inviter = Member::new(10);
    world.join(&inviter, creator, "code");
    let own_invite = world.author(&inviter, invite(2, "code2"), Deps::Frontier);
    (world, inviter, own_invite)
}

/// IC3: with a usable hint, a join cites a `Checkpoint` op carrying its
/// digest (the lowest by `(lamport, op_hash)`) in place of the creator's
/// seq-4 op, so the whole cut is in its causal past.
#[test]
fn a_join_cites_a_checkpoint_op_of_its_hint() {
    let creator = Member::new(1);
    let (mut world, inviter, own_invite) = with_member_inviter(&creator);
    let body = world.checkpoint_body();
    let checkpoint = world.endorse(&creator, body);
    let outcome = merge::merge_outcome(&world.cert, &world.set.ops);
    assert_eq!(
        outcome.hints.latest().map(|h| h.op),
        Some(checkpoint.hash())
    );
    let joined = world.join(&Member::new(20), &inviter, "code2");
    assert_eq!(
        dep_hashes(&joined),
        BTreeSet::from([checkpoint.hash(), own_invite.hash()])
    );
    assert!(world.state.members.contains_key(&Member::new(20).author()));
}

/// Under a checkpoint equivocation (two digests, each with a strict
/// majority, for one cut and one `prevCheckpoint`) no hint is selected,
/// so a join cites the creator's seq-4 op, never a `Checkpoint` op.
#[test]
fn an_equivocated_community_uses_no_hint() {
    let creator = Member::new(1);
    let (mut world, inviter, own_invite) = with_member_inviter(&creator);
    let base = world.op_at(&creator, GENESIS_LAST_SEQ);
    let body = world.checkpoint_body();
    world.endorse(&creator, body.clone());
    let mut other = body;
    other.round = 1;
    other.state_root = [9; 32];
    world.endorse(&creator, other);
    let outcome = merge::merge_outcome(&world.cert, &world.set.ops);
    assert_eq!(outcome.hints.equivocations.len(), 1, "{:?}", outcome.hints);
    let joined = world.join(&Member::new(20), &inviter, "code2");
    assert_eq!(
        dep_hashes(&joined),
        BTreeSet::from([base, own_invite.hash()])
    );
}
