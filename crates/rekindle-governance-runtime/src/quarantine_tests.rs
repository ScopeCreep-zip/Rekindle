//! D15: a validly signed op of a kind this reader cannot interpret
//! quarantines the community with a reason; it is held (never dropped),
//! never applied, and the state is the merge without it.

use rekindle_codec::governance_op::{op_unsigned_bytes, seal_unsigned, Uninterpretable};
use rekindle_governance::merge;
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_secrets::ed25519_dalek::Signer;
use rekindle_types::governance::OpRef;

use crate::event::GovernanceRuntimeEvent;
use crate::op_log::{fates, held_from_encoded, verify_held, Deps, HeldOp, OpFate};
use crate::quarantine::newly_uninterpretable;
use crate::test_member::Member;
use crate::test_world::{invite, World, ANCHOR};

/// `unsigned` with its body's union discriminant (the first 16 bits of the
/// body's data section; the body is the root's pointer 3) set to `arm`.
fn with_arm(unsigned: &[u8], arm: u16) -> Vec<u8> {
    let word = |i: usize| u64::from_le_bytes(unsigned[i * 8..i * 8 + 8].try_into().unwrap());
    let target = |at: usize| {
        let ptr = word(at);
        let offset = usize::try_from((ptr as u32 as i32) >> 2).unwrap();
        (at + 1 + offset, usize::from((ptr >> 32) as u16))
    };
    let (root, root_data) = target(0);
    let (body, _) = target(root + root_data + 3);
    let mut out = unsigned.to_vec();
    out[body * 8..body * 8 + 2].copy_from_slice(&arm.to_le_bytes());
    out
}

/// `author`'s next op with its body arm replaced by one beyond the schema,
/// signed by `signer` over the bytes it carries, as the op store holds a
/// fetched op.
fn newer_kind_op(world: &mut World, author: &Member, signer: &Member) -> HeldOp {
    let (op, _) = world.build(author, invite(9, "newer"), Deps::Frontier);
    let unsigned = with_arm(&op_unsigned_bytes(op.op()), 0x7fff);
    let sig = (signer.pseudonym)
        .sign(&op_signing_bytes(ANCHOR, &unsigned))
        .to_bytes();
    let op_ref = OpRef {
        op_hash: op_hash(&unsigned, &sig),
        author: author.author(),
        author_record: author.record(),
        author_seq: op.op().seq,
    };
    held_from_encoded(&op_ref, seal_unsigned(&unsigned, &sig).unwrap())
        .expect("an op of an unknown kind is held, not dropped")
}

#[test]
fn a_members_op_of_an_unknown_kind_quarantines_the_community() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let unknown = newer_kind_op(&mut world, &creator, &creator);
    let mut held = world.held.clone();
    held.push(unknown.clone());

    let set = verify_held(ANCHOR, &held);
    assert!(set.quarantined.contains(&unknown.hash), "never applied");
    let outcome = merge::merge_outcome(&world.cert, &set.ops);
    assert_eq!(outcome.state, world.state, "the merge without it");
    let verdict = fates(&set, &outcome);
    assert!(verdict
        .iter()
        .any(|v| v.hash == unknown.hash && v.fate == OpFate::Quarantined));

    let events = newly_uninterpretable(ANCHOR, &held, &outcome.state);
    assert_eq!(events.len(), 1, "the community is quarantined: {events:?}");
    let GovernanceRuntimeEvent::CommunityQuarantined {
        community_id,
        op,
        author,
        what,
    } = &events[0]
    else {
        panic!("not CommunityQuarantined");
    };
    assert_eq!(
        (community_id.as_str(), *op, author, what),
        (
            ANCHOR,
            unknown.hash,
            &creator.author(),
            &Uninterpretable::Kind(0x7fff)
        )
    );

    // Held quarantined after the rebuild records its fate: shown once.
    let mut kept = held;
    kept.last_mut().unwrap().fate = OpFate::Quarantined;
    assert!(newly_uninterpretable(ANCHOR, &kept, &outcome.state).is_empty());
}

#[test]
fn an_unknown_op_not_signed_by_a_member_quarantines_nothing() {
    let creator = Member::new(1);
    let outsider = Member::new(9);
    let mut world = World::new(&creator);
    let forged = newer_kind_op(&mut world, &creator, &outsider);
    let foreign = newer_kind_op(&mut world, &outsider, &outsider);
    for op in [forged, foreign] {
        let mut held = world.held.clone();
        held.push(op);
        assert!(newly_uninterpretable(ANCHOR, &held, &world.state).is_empty());
    }
}
