//! D15: a validly signed op of a kind this reader cannot interpret
//! quarantines the community with a reason; it is held (never dropped),
//! never applied, and the state is the merge without it.

use rekindle_codec::governance_op::{op_unsigned_bytes, seal_op, Uninterpretable};
use rekindle_governance::merge;
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_secrets::ed25519_dalek::Signer;
use rekindle_types::governance::OpRef;

use crate::event::GovernanceRuntimeEvent;
use crate::op_log::{fates, held_from_encoded, verify_held, Deps, HeldOp, OpFate};
use crate::quarantine::newly_uninterpretable;
use crate::test_member::Member;
use crate::test_world::{invite, World, ANCHOR};

/// The signed wire op of canonical unsigned `unsigned` once its body's
/// union discriminant (the first 16 bits of the body's data section; the
/// body is the root's pointer 3) is `arm`: the patched bytes, signed by
/// `signer`, sealed by the codec. Returns (patched unsigned, sig, wire).
fn newer_kind_wire(unsigned: &[u8], arm: u16, signer: &Member) -> (Vec<u8>, [u8; 64], Vec<u8>) {
    let pointee = |at: usize| {
        let p = &unsigned[at * 8..at * 8 + 8];
        let words = i32::from_le_bytes(p[0..4].try_into().unwrap()) >> 2;
        let data = u16::from_le_bytes(p[4..6].try_into().unwrap());
        (at + 1 + usize::try_from(words).unwrap(), usize::from(data))
    };
    let (root_at, root_data) = pointee(0);
    let body_at = pointee(root_at + root_data + 3).0;
    let mut patched = unsigned.to_vec();
    patched[body_at * 8..body_at * 8 + 2].copy_from_slice(&arm.to_le_bytes());
    let sig = (signer.pseudonym)
        .sign(&op_signing_bytes(ANCHOR, &patched))
        .to_bytes();
    let wire = seal_op(&patched, &sig).expect("the codec seals the patched op");
    (patched, sig, wire)
}

/// `author`'s next op with its body arm replaced by one beyond the schema,
/// signed by `signer` over the bytes it carries, as the op store holds a
/// fetched op.
fn newer_kind_op(world: &mut World, author: &Member, signer: &Member) -> HeldOp {
    let (op, _) = world.build(author, invite(9, "newer"), Deps::Frontier);
    let (unsigned, sig, wire) = newer_kind_wire(&op_unsigned_bytes(op.op()), 0x7fff, signer);
    let op_ref = OpRef {
        op_hash: op_hash(&unsigned, &sig),
        device_key: author.author().0,
        author: author.author(),
        author_record: author.record(),
        author_seq: op.op().seq,
    };
    held_from_encoded(&op_ref, wire).expect("an op of an unknown kind is held, not dropped")
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
