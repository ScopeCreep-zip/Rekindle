//! Shallow joins (plan E3.2 S7c.0): a join cites the creator's last
//! genesis op and the inviter's latest op, never the previous joiner, so
//! the DAG's depth stops growing with the number of joins; a fold join
//! cites at most 64 deps; a re-join cites its own chain and its removal.

use super::*;
use crate::op_log::{
    author_record, build_op, causal_past, frontier_refs, held, verify_held, Deps, HeldOp, OpFate,
};
use crate::origin::genesis_entries;
use crate::test_member::Member;
use rekindle_codec::governance_op::{decode_op, encode_op};
use rekindle_governance::merge;
use rekindle_governance::op::invite_proof_bytes;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::Signer;
use rekindle_secrets::member_record::sign_member_record_binding;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry, InviteProof};
use rekindle_types::id::ChannelId;

const ANCHOR: &str = "VLD0:anchor";

impl Member {
    fn joined(&self, code: &str) -> GovernanceEntry {
        let record = self.record();
        let sig = self
            .pseudonym
            .sign(&invite_proof_bytes(ANCHOR, code, &record))
            .to_bytes()
            .to_vec();
        let owner_proof =
            sign_member_record_binding(&self.owner.to_bytes(), ANCHOR, &self.author().0, &record);
        GovernanceEntry::MemberJoined {
            member_record: record,
            member_record_owner: self.owner_text(),
            invite_proof: Some(InviteProof {
                code_hash: code.into(),
                sig,
            }),
            owner_proof: owner_proof.to_vec(),
        }
    }
}

fn invite(id: u8, code: &str) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [id; 16],
        code_hash: code.into(),
        max_uses: 100,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    }
}

/// A community's held ops, re-verified and re-merged after each op.
struct World {
    cert: GenesisCert,
    held: Vec<HeldOp>,
    set: VerifiedSet,
    state: GovernanceState,
    clock: u64,
}

impl World {
    /// The creator's five genesis ops, then its invite `code`.
    fn new(creator: &Member) -> Self {
        let cert = GenesisCert {
            name: "c".into(),
            policy: AdmissionMode::Open,
            initial_roles: Vec::new(),
            creator_pseudonym: creator.author(),
            creator_member_record: creator.record(),
            creator_member_record_owner: creator.owner_text(),
            created_at: 1,
        };
        let mut world = Self {
            cert,
            held: Vec::new(),
            set: VerifiedSet::default(),
            state: GovernanceState::default(),
            clock: 1,
        };
        let genesis = genesis_entries("c", &creator.author(), ChannelId([2; 16]), "VLD0:ch");
        for body in genesis {
            world.author(creator, body, Deps::Frontier);
        }
        world.author(creator, invite(1, "code"), Deps::Frontier);
        world
    }

    /// `member` authors `body` with `deps` after everything held.
    fn author(&mut self, member: &Member, body: GovernanceEntry, deps: Deps) -> VerifiedOp {
        let op = self.build(member, body, deps);
        self.hold(&op.0, op.1);
        op.0
    }

    fn build(
        &mut self,
        member: &Member,
        body: GovernanceEntry,
        deps: Deps,
    ) -> (VerifiedOp, Vec<u8>) {
        self.clock += 1;
        let (cert, state, set) = (&self.cert, &self.state, &self.set);
        build_op(
            ANCHOR,
            cert,
            state,
            set,
            &member.pseudonym,
            body,
            deps,
            self.clock,
        )
        .unwrap()
    }

    fn hold(&mut self, op: &VerifiedOp, encoded: Vec<u8>) {
        let record = match op.body() {
            GovernanceEntry::MemberJoined { member_record, .. } => member_record.clone(),
            _ => author_record(&self.cert, &self.state, op.author()).unwrap(),
        };
        self.held.push(held(op, record, encoded, OpFate::Applied));
        self.set = verify_held(ANCHOR, &self.held);
        assert!(self.set.pending.is_empty() && self.set.quarantined.is_empty());
        self.state = merge::merge(&self.cert, &self.set.ops);
    }

    /// `member`'s join on `inviter`'s invite `code`, with shallow deps.
    fn join(&mut self, member: &Member, inviter: &Member, code: &str) -> VerifiedOp {
        let deps = join_deps(&self.cert, &self.set, &member.author(), &inviter.author()).unwrap();
        self.author(member, member.joined(code), Deps::Join(deps))
    }

    /// The hashes of `author`'s op at `seq`.
    fn op_at(&self, author: &Member, seq: u64) -> OpHash {
        let key = author.author();
        self.set
            .ops
            .iter()
            .find(|op| *op.author() == key && op.op().seq == seq)
            .unwrap()
            .hash()
    }
}

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
        let deps = join_deps(&world.cert, &world.set, &member.author(), &creator.author()).unwrap();
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
fn a_revocation_outside_the_inviters_past_leaves_a_join_concurrent() {
    // Accepted (C5): the join sees only the inviter's chain and the base.
    let (mut world, inviter) =
        with_member_invite(|_| GovernanceEntry::InviteRevoked { invite_id: [2; 16] });
    let joiner = Member::new(20);
    world.join(&joiner, &inviter, "code2");
    assert!(world.state.members.contains_key(&joiner.author()));
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
    let deps = join_deps(&world.cert, &world.set, &joiner.author(), &inviter.author()).unwrap();
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
