//! The joiner's walk above a checkpoint's cut (plan E3.2 S7c.5), over a
//! signed community whose ops verify as any reader verifies them.

use std::collections::{BTreeSet, HashMap};

use rekindle_governance::checkpoint::{causal_past, state_preimage, CutSnapshot};
use rekindle_governance::checksum::roster_checksum;
use rekindle_governance::merge::{merge_cut, merge_outcome};
use rekindle_types::governance::{Checkpoint, GovernanceEntry, OpRef, RosterChecksum};

use super::*;
use crate::join_deps::join_deps;
use crate::op_log::{frontier_refs, Deps, VerifiedSet};
use crate::test_member::Member;
use crate::test_world::{invite, World, ANCHOR};

/// The creator, a member inviter holding its own invite `code2`, and a
/// height-1 checkpoint over the frontier then, endorsed by the creator
/// (the cert's creator set is the height-1 electorate).
fn world() -> (World, Member, Member, VerifiedOp) {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let inviter = Member::new(10);
    world.join(&inviter, &creator, "code");
    world.author(&inviter, invite(2, "code2"), Deps::Frontier);
    world.author(
        &creator,
        rekindle_governance::fixture::meta("x"),
        Deps::Frontier,
    );
    let body = world.checkpoint_body();
    let cp = world.endorse(&creator, body);
    (world, creator, inviter, cp)
}

/// The snapshot at the cut whose heads are `heads`: the body a writer
/// would endorse over them (`Replay::body`) and the preimage of its
/// `stateRoot`.
fn snapshot_at(world: &World, heads: &[OpRef]) -> (Checkpoint, CutSnapshot) {
    let replay = rekindle_governance::checkpoint::Replay {
        cert: &world.cert,
        ops: &world.set.ops,
        community_id: [7; 32],
    };
    let body = replay.body(None, heads).expect("a body");
    let (state, cut) = merge_cut(&world.cert, &causal_past(&world.set.ops, &body.cut));
    let snap = CutSnapshot::decode(&body, &state_preimage(&state, &cut)).expect("canonical");
    (body, snap)
}

fn cp_body(cp: &VerifiedOp) -> &Checkpoint {
    match cp.body() {
        GovernanceEntry::Checkpoint(c) => c,
        _ => unreachable!("a checkpoint op"),
    }
}

/// The world's frontier, as its holder publishes it.
fn frontier(world: &World) -> Frontier {
    Frontier {
        heads: frontier_refs(&world.cert, &world.state, &world.set),
        roster_checksum: roster_checksum(&world.state),
    }
}

/// Every held op by hash, served by locator; each fetch is counted.
struct Store {
    ops: HashMap<OpHash, HeldOp>,
    fetched: parking_lot::Mutex<Vec<OpHash>>,
}

impl Store {
    fn of(world: &World) -> Self {
        Self {
            ops: world.held.iter().map(|h| (h.hash, h.clone())).collect(),
            fetched: parking_lot::Mutex::new(Vec::new()),
        }
    }

    fn fetch(
        &self,
        r: &OpRef,
    ) -> std::future::Ready<Result<Option<HeldOp>, GovernanceRuntimeError>> {
        self.fetched.lock().push(r.op_hash);
        std::future::ready(Ok(self.ops.get(&r.op_hash).cloned()))
    }

    fn fetched(&self) -> BTreeSet<OpHash> {
        self.fetched.lock().iter().copied().collect()
    }
}

/// Resume from `snaps` (newest first) over `store`, from `frontier`.
fn resume(
    world: &World,
    store: &Store,
    snaps: &[CutSnapshot],
    frontier: &Frontier,
) -> Option<Resumed> {
    futures::executor::block_on(resume_newest(&world.cert, snaps, frontier, |i| {
        gather(ANCHOR, &world.cert, &snaps[i], &frontier.heads, |r| {
            store.fetch(&r)
        })
    }))
    .expect("no fetch fails")
}

/// The resumed merge reaches what the full merge reaches: the state, and
/// every post-cut op's fate (`merge_from` reports the cut's power fates
/// and the post-cut ones).
fn assert_matches_full(world: &World, resumed: &Resumed) {
    let full = merge_outcome(&world.cert, &world.set.ops);
    assert_eq!(resumed.outcome.state, full.state);
    assert_eq!(resumed.outcome.quarantined, full.quarantined);
    for op in &resumed.gathered.rest {
        let h = op.hash();
        assert_eq!(
            resumed.outcome.applied.contains(&h),
            full.applied.contains(&h),
            "a post-cut fate"
        );
        assert_eq!(
            resumed.outcome.refused.contains(&h),
            full.refused.contains(&h)
        );
    }
}

/// The joiner walks only above the cut: every cut op it fetches is one a
/// post-cut op cites, an applied cut power op, or one such a power op
/// cites; the rest of the cut is never fetched, and the merge from the
/// snapshot is the full merge's.
#[test]
fn a_joiner_starts_from_a_hint_and_skips_only_the_cut() {
    let (mut world, _, inviter, cp) = world();
    world.author(
        &inviter,
        rekindle_governance::fixture::meta("p"),
        Deps::Frontier,
    );
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let resumed = resume(
        &world,
        &store,
        std::slice::from_ref(&snap),
        &frontier(&world),
    )
    .expect("on top");
    assert_eq!(resumed.index, 0);
    assert_matches_full(&world, &resumed);

    let by_hash: HashMap<OpHash, &VerifiedOp> =
        world.set.ops.iter().map(|o| (o.hash(), o)).collect();
    let cut: BTreeSet<OpHash> = causal_past(&world.set.ops, &cp_body(&cp).cut)
        .iter()
        .map(VerifiedOp::hash)
        .collect();
    let rest: Vec<&VerifiedOp> = (world.set.ops.iter())
        .filter(|o| !cut.contains(&o.hash()))
        .collect();
    let power: Vec<OpHash> = (snap.cut.power.iter())
        .filter(|(_, applied)| *applied)
        .map(|(c, _)| c.hash)
        .collect();
    let mut allowed: BTreeSet<OpHash> = power.iter().copied().collect();
    for op in rest.iter().copied().chain(power.iter().map(|h| by_hash[h])) {
        allowed.extend(op.op().deps.iter().map(|d| d.op_hash));
    }
    let fetched_cut: BTreeSet<OpHash> = store.fetched().intersection(&cut).copied().collect();
    assert!(fetched_cut.is_subset(&allowed), "only the cut ops needed");
    assert!(fetched_cut.len() < cut.len(), "some of the cut is skipped");
    let rest_hashes: BTreeSet<OpHash> = rest.iter().map(|o| o.hash()).collect();
    let resumed_rest: BTreeSet<OpHash> =
        resumed.gathered.rest.iter().map(VerifiedOp::hash).collect();
    assert_eq!(resumed_rest, rest_hashes, "every post-cut op is walked");
}

/// A post-cut op may cite a cut op other than its author's top: that op
/// is fetched by its locator and checked, and the citing op verifies with
/// the lamport the full verification gives it.
#[test]
fn a_cited_non_top_cut_op_is_fetched_and_verified() {
    let (mut world, creator, _, cp) = world();
    let old = world.op_at(&creator, 2);
    let q = world.author(
        &creator,
        rekindle_governance::fixture::meta("q"),
        Deps::Cited(vec![cp.hash(), old]),
    );
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let resumed = resume(&world, &store, &[snap], &frontier(&world)).expect("on top");
    assert!(resumed.gathered.cut_fetched.contains(&old));
    let ours = (resumed.gathered.rest.iter())
        .find(|o| o.hash() == q.hash())
        .expect("q verifies over the cut");
    assert_eq!(ours.lamport(), q.lamport());
    assert!(resumed.outcome.applied.contains(&q.hash()));
    assert_matches_full(&world, &resumed);
}

/// A fetched cut op is taken only with its locator's hash, author and
/// seq, its author's signature under this community, and a seq inside
/// the cut.
#[test]
fn a_cut_op_is_taken_only_when_hash_signature_and_seq_check() {
    let (mut world, creator, inviter, cp) = world();
    let after = world.author(
        &inviter,
        rekindle_governance::fixture::meta("p"),
        Deps::Frontier,
    );
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let old = world.op_at(&creator, 2);
    let held_old = &store.ops[&old];
    let r = world
        .set
        .ops
        .iter()
        .find(|o| o.hash() == old)
        .unwrap()
        .op_ref("VLD0:x");
    assert!(cut_fact(ANCHOR, &snap, &r, held_old).is_some());
    let mut other_hash = r.clone();
    other_hash.op_hash = cp.hash();
    assert!(
        cut_fact(ANCHOR, &snap, &other_hash, held_old).is_none(),
        "hash"
    );
    let mut other_seq = r.clone();
    other_seq.author_seq += 1;
    assert!(
        cut_fact(ANCHOR, &snap, &other_seq, held_old).is_none(),
        "seq"
    );
    assert!(
        cut_fact("VLD0:other", &snap, &r, held_old).is_none(),
        "signature"
    );
    let above = after.op_ref("VLD0:x");
    assert!(
        cut_fact(ANCHOR, &snap, &above, &store.ops[&after.hash()]).is_none(),
        "in the cut"
    );
}

/// Ops outside the cut are walked and merged whoever wrote them: a join
/// of a member the cut's directory does not hold, and a removal, land as
/// the full merge lands them, never quarantined.
#[test]
fn an_op_outside_the_cut_is_merged_never_quarantined() {
    let (mut world, creator, inviter, cp) = world();
    let newcomer = Member::new(20);
    let joined = world.join(&newcomer, &inviter, "code2");
    let removal = world.author(
        &creator,
        GovernanceEntry::MemberRemoved {
            target: newcomer.author(),
            reason: None,
            heads: Vec::new(),
            target_seq_frontier: 0,
        },
        Deps::Frontier,
    );
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let resumed = resume(&world, &store, &[snap], &frontier(&world)).expect("on top");
    for op in [&joined, &removal] {
        assert!(resumed.outcome.applied.contains(&op.hash()));
        assert!(!(resumed.outcome.quarantined.iter()).any(|(h, _)| *h == op.hash()));
    }
    assert_matches_full(&world, &resumed);
}

/// The cut's permissions and invites hold above it (IC6): a post-cut op
/// its author has no permission for in the snapshot's state is refused,
/// and an invite revoked in the cut is not live in the resumed state, so
/// `join_flow::check_invite` refuses a join on it while one created above
/// the cut stays live.
#[test]
fn post_cut_permission_and_invite_checks_run_on_the_snapshot() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let inviter = Member::new(10);
    world.join(&inviter, &creator, "code");
    world.author(
        &creator,
        GovernanceEntry::InviteRevoked { invite_id: [1; 16] },
        Deps::Frontier,
    );
    let body = world.checkpoint_body();
    let cp = world.endorse(&creator, body);
    let renamed = world.author(
        &inviter,
        rekindle_governance::fixture::meta("r"),
        Deps::Frontier,
    );
    world.author(&creator, invite(3, "code3"), Deps::Frontier);
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let resumed = resume(&world, &store, &[snap], &frontier(&world)).expect("on top");
    assert!(
        resumed.outcome.refused.contains(&renamed.hash()),
        "no permission"
    );
    let state = &resumed.outcome.state;
    assert!(
        state.live_invite("code", 10).is_none(),
        "revoked in the cut"
    );
    assert!(state.live_invite("code3", 10).is_some(), "created above it");
    assert_matches_full(&world, &resumed);
}

/// Two cuts, `a` and the later `b`, and a power op `r` above `a` but
/// concurrent with `b` (it cites `a`'s head and not `b`'s): resuming from
/// `b` is not on top (a power op beside the cut could reorder its power
/// phase, plan E3.2 S7c.8 (3)), so the older `a` is used; with `b` alone
/// the caller replays in full.
#[test]
fn a_not_on_top_op_moves_to_the_older_hint_then_to_full_replay() {
    let (mut world, creator, inviter, cp) = world();
    let (_, a) = snapshot_at(&world, &cp_body(&cp).cut);
    world.author(
        &creator,
        rekindle_governance::fixture::meta("b"),
        Deps::Frontier,
    );
    let b_heads = frontier_refs(&world.cert, &world.state, &world.set);
    let approval = GovernanceEntry::MemberApproved {
        target: Member::new(40).author(),
    };
    let r = world.author(&inviter, approval, Deps::Cited(vec![cp.hash()]));
    let (_, b) = snapshot_at(&world, &b_heads);
    let store = Store::of(&world);
    let front = frontier(&world);
    let resumed = resume(&world, &store, &[b.clone(), a], &front).expect("a is on top");
    assert_eq!(resumed.index, 1, "the older hint");
    assert!((resumed.gathered.rest.iter()).any(|o| o.hash() == r.hash()));
    assert_matches_full(&world, &resumed);
    assert!(
        resume(&world, &store, &[b], &front).is_none(),
        "full replay"
    );
}

/// A join citing an older checkpoint than the hint in use lacks a head of
/// that hint's cut: it is non-power, and every applied cut power op is in
/// its past, so the joiner resumes from the newer hint (plan E3.2 S7c.8
/// (3); it used the older one before), and the join lands as the full
/// merge lands it.
#[test]
fn a_join_citing_an_older_checkpoint_is_replayed() {
    let (mut world, creator, inviter, cp1) = world();
    let (_, h1) = snapshot_at(&world, &cp_body(&cp1).cut);
    world.author(
        &creator,
        rekindle_governance::fixture::meta("y"),
        Deps::Frontier,
    );
    let heads = frontier_refs(&world.cert, &world.state, &world.set);
    let (_, h2) = snapshot_at(&world, &heads);
    let newcomer = Member::new(20);
    let own_invite = world.op_at(&inviter, 1);
    let joined = world.author(
        &newcomer,
        newcomer.joined("code2"),
        Deps::Join(vec![cp1.hash(), own_invite]),
    );
    let store = Store::of(&world);
    let resumed = resume(&world, &store, &[h2, h1], &frontier(&world)).expect("h2 is on top");
    assert_eq!(resumed.index, 0);
    assert!(resumed.outcome.applied.contains(&joined.hash()));
    assert_matches_full(&world, &resumed);
}

/// A resumed state is compared with the inviter's roster checksum at
/// equal heads: a mismatch drops the hint (the caller replays in full).
#[test]
fn a_resumed_state_that_misses_the_ring_checksum_is_replayed() {
    let (world, _, _, cp) = world();
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let mut front = frontier(&world);
    assert!(resume(&world, &store, std::slice::from_ref(&snap), &front).is_some());
    front.roster_checksum = RosterChecksum([9; 32]);
    assert!(resume(&world, &store, &[snap], &front).is_none());
}

/// After a resume the joiner holds only the post-cut ops: an inviter with
/// none has its invite in the cut, which the hint's `Checkpoint` op has
/// in its past, so the join cites that op alone.
#[test]
fn a_resumed_join_whose_inviter_has_no_post_cut_op_cites_the_hint_alone() {
    let (world, _, inviter, cp) = world();
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let resumed = resume(&world, &store, &[snap], &frontier(&world)).expect("on top");
    let set = VerifiedSet {
        ops: resumed.gathered.rest.clone(),
        ..VerifiedSet::default()
    };
    let digest = rekindle_governance::checkpoint::checkpoint_digest(cp_body(&cp));
    let deps = join_deps(
        &world.cert,
        &set,
        &resumed.outcome,
        Some(&digest),
        &Member::new(20).author(),
        &inviter.author(),
    )
    .expect("the hint covers the inviter's invite");
    assert_eq!(deps, vec![cp.hash()]);
}

/// Two digests for one cut, each endorsed by a strict majority (the
/// creator alone at height 1, in two rounds), above the cut of the hint in
/// use: a checkpoint equivocation, so the joiner uses no hint.
#[test]
fn an_equivocated_community_uses_no_hint() {
    let (mut world, creator, _, cp) = world();
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    let store = Store::of(&world);
    let resumed = resume(
        &world,
        &store,
        std::slice::from_ref(&snap),
        &frontier(&world),
    )
    .expect("on top");
    assert!(!equivocates(&world.cert, None, 1, &resumed.gathered.rest));
    let mut other = cp_body(&cp).clone();
    other.round = 1;
    other.state_root = [9; 32];
    world.endorse(&creator, other);
    let store = Store::of(&world);
    let resumed = resume(&world, &store, &[snap], &frontier(&world)).expect("on top");
    assert!(equivocates(&world.cert, None, 1, &resumed.gathered.rest));
}

/// A join racing the checkpoint (plan E3.2 S7c.8 (3)): its deps, taken
/// before the checkpoint, are the creator's last genesis op (not its top
/// in the cut) and the inviter's invite, so it is beside the cut. The
/// joiner after it resumes from the hint, the racing join verifies with
/// the lamport the full verification gives it (its cited cut op's exact
/// lamport, from that op's in-cut past), and the merge is the full one.
#[test]
fn a_join_racing_the_checkpoint_resumes_with_its_exact_lamport() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let inviter = Member::new(10);
    world.join(&inviter, &creator, "code");
    world.author(&inviter, invite(2, "code2"), Deps::Frontier);
    world.author(
        &creator,
        rekindle_governance::fixture::meta("x"),
        Deps::Frontier,
    );
    let racer = Member::new(30);
    let racing_deps = world.deps_of(&racer, &inviter);
    let body = world.checkpoint_body();
    let cp = world.endorse(&creator, body);
    let raced = world.author(&racer, racer.joined("code2"), Deps::Join(racing_deps));
    world.join(&Member::new(31), &inviter, "code2");
    let (_, snap) = snapshot_at(&world, &cp_body(&cp).cut);
    assert!(
        !(snap.heads.iter()).all(|h| raced.op().deps.iter().any(|d| d.op_hash == *h)),
        "the racing join lacks a cut head"
    );
    let store = Store::of(&world);
    let resumed = resume(&world, &store, &[snap], &frontier(&world)).expect("on top");
    let ours = (resumed.gathered.rest.iter())
        .find(|o| o.hash() == raced.hash())
        .expect("the racing join verifies over the cut");
    assert_eq!(ours.lamport(), raced.lamport());
    assert!(resumed.outcome.applied.contains(&raced.hash()));
    assert_matches_full(&world, &resumed);
}
