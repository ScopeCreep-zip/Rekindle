//! The checkpoint rows a rebuild writes (plan E3.2 S7c.6).

use rekindle_governance::checkpoint::checkpoint_digest;
use rekindle_governance::merge::merge_outcome;

use super::*;
use crate::op_log::Deps;
use crate::test_member::Member;
use crate::test_world::{invite, World};

fn facts(world: &World) -> Vec<CheckpointFact> {
    checkpoint_facts(&world.set.ops, &merge_outcome(&world.cert, &world.set.ops))
}

/// A digest short of a majority is `Proposed`, a majority makes it
/// `Usable` (its op the lowest carrying it), and a second majority digest
/// for the same cut makes both `Equivocated`, so the snapshot kept for the
/// first is not read again (`usable_snapshot` reads only usable rows).
#[test]
fn a_checkpoint_row_is_proposed_then_usable_then_equivocated() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    let member = Member::new(10);
    world.join(&member, &creator, "code");
    world.author(&member, invite(2, "code2"), Deps::Frontier);
    let body = world.checkpoint_body();
    let digest = checkpoint_digest(&body);
    let first = world.endorse(&member, body.clone());
    let row = |state, op| CheckpointFact {
        height: 1,
        round: 0,
        digest,
        op,
        dir_size: body.dir_size,
        state,
    };
    assert_eq!(
        facts(&world),
        vec![row(HintState::Proposed, first.hash())],
        "the member is outside the height-1 electorate (the cert's creator set)"
    );
    let by_creator = world.endorse(&creator, body.clone());
    let lowest = [&first, &by_creator]
        .into_iter()
        .min_by_key(|op| op.stamp())
        .map(VerifiedOp::hash)
        .unwrap();
    assert_eq!(facts(&world), vec![row(HintState::Usable, lowest)]);
    let mut other = body.clone();
    other.round = 1;
    other.state_root = [9; 32];
    let second = world.endorse(&creator, other.clone());
    assert_eq!(
        facts(&world),
        vec![
            row(HintState::Equivocated, lowest),
            CheckpointFact {
                round: 1,
                digest: checkpoint_digest(&other),
                op: second.hash(),
                ..row(HintState::Equivocated, second.hash())
            },
        ]
    );
}
