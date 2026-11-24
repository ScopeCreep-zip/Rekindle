//! The walk from the anchor (plan E3.2 S7d-keys K5): a member's ops that
//! no other member's frontier cites are reached through its own record,
//! which only the merged `MemberJoined` names.

use std::collections::BTreeMap;

use rekindle_types::governance::{Frontier, OpRef};

use super::walk_records_with;
use crate::op_log::{verify_held, Deps, HeldOp};
use crate::op_walk::walk_with;
use crate::test_member::Member;
use crate::test_world::{invite, World, ANCHOR};
use crate::verify_cache::OpVerifyCache;

fn op_ref(op: &HeldOp) -> OpRef {
    OpRef {
        op_hash: op.hash,
        author: op.author.clone(),
        author_record: op.author_record.clone(),
        author_seq: op.seq,
        device_key: op.device_key,
    }
}

#[tokio::test]
async fn every_member_record_the_walk_names_is_read() {
    let (a, b) = (Member::new(1), Member::new(2));
    let mut world = World::new(&a);
    // The creator merged b's join and has not read b since: its frontier
    // cites b's `MemberJoined`, never b's later op.
    world.join(&b, &a, "code");
    let a_head = op_ref(world.held.last().unwrap());
    world.author(&b, invite(2, "code2"), Deps::Frontier);
    let b_head = op_ref(world.held.last().unwrap());
    let frontiers: BTreeMap<String, Vec<OpRef>> =
        BTreeMap::from([(a.record(), vec![a_head]), (b.record(), vec![b_head])]);
    let all = world.held.clone();
    let (frontiers, all) = (&frontiers, &all);
    let (held, _set, merged, records) = walk_records_with(
        &world.cert,
        |record| async move {
            Ok(frontiers.get(&record).map(|heads| Frontier {
                heads: heads.clone(),
                roster_checksum: rekindle_types::governance::RosterChecksum([0; 32]),
                fork_proof: Vec::new(),
                fork_refs: Vec::new(),
            }))
        },
        |heads, mut held: Vec<HeldOp>| async move {
            let walked = walk_with(ANCHOR, &heads, &OpVerifyCache::default(), |r| async move {
                Ok(all.iter().find(|op| op.hash == r.op_hash).cloned())
            })
            .await?;
            for op in walked {
                if !held.iter().any(|h| h.hash == op.hash) {
                    held.push(op);
                }
            }
            Ok(held)
        },
        |held| verify_held(ANCHOR, held),
    )
    .await
    .unwrap();
    assert_eq!(
        held.len(),
        world.held.len(),
        "every op is reached: {records:?} {:?}",
        merged
            .state
            .members
            .values()
            .map(|m| &m.member_record)
            .collect::<Vec<_>>()
    );
    assert!(merged.state.invites.contains_key(&[2; 16]));
    assert!(merged.state.members.contains_key(&b.author()));
    assert!(records.contains(&b.record()));
}
