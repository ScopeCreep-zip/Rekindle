//! `check_invite` answers the typed reason the inviter's merge would give
//! (`rekindle_types::join_refusal`), from the op DAG the join walked.

use rekindle_governance::merge;
use rekindle_secrets::invite::hash_invite_code;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::join_refusal::JoinRefusal;

use super::check_invite;
use crate::error::GovernanceRuntimeError;
use crate::op_log::Deps;
use crate::test_member::Member;
use crate::test_world::World;

/// The join's check of `code` against everything `world` holds, at `now`.
fn check(world: &World, code: &str, now: u64) -> Result<(), JoinRefusal> {
    let outcome = merge::merge_outcome(&world.cert, &world.set.ops);
    let walked = (world.set.ops.as_slice(), outcome.applied.as_slice());
    match check_invite(&outcome.state, walked, &hash_invite_code(code), now) {
        Ok(_) => Ok(()),
        Err(GovernanceRuntimeError::InviteRefused(reason)) => Err(reason),
        Err(other) => panic!("an untyped refusal: {other}"),
    }
}

fn limited(id: u8, code: &str, max_uses: u32, expires_at: Option<u64>) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [id; 16],
        code_hash: hash_invite_code(code),
        max_uses,
        expires_at,
        secrets_record_key: "VLD0:s".into(),
    }
}

#[test]
fn a_live_invite_names_its_creator() {
    let creator = Member::new(1);
    let world = World::new(&creator);
    let outcome = merge::merge_outcome(&world.cert, &world.set.ops);
    let walked = (world.set.ops.as_slice(), outcome.applied.as_slice());
    let inviter =
        check_invite(&outcome.state, walked, &hash_invite_code("code"), 10).expect("live");
    assert_eq!(inviter, creator.author());
}

#[test]
fn a_used_up_invite_is_invite_used_up() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    world.author(&creator, limited(2, "once", 1, None), Deps::Frontier);
    world.join(&Member::new(10), &creator, "once");
    assert_eq!(check(&world, "once", 10), Err(JoinRefusal::InviteUsedUp));
}

#[test]
fn a_revoked_invite_is_no_longer_valid() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    world.author(
        &creator,
        GovernanceEntry::InviteRevoked { invite_id: [1; 16] },
        Deps::Frontier,
    );
    assert_eq!(
        check(&world, "code", 10),
        Err(JoinRefusal::InviteNoLongerValid)
    );
}

#[test]
fn an_expired_invite_is_no_longer_valid() {
    let creator = Member::new(1);
    let mut world = World::new(&creator);
    world.author(&creator, limited(2, "soon", 5, Some(20)), Deps::Frontier);
    assert_eq!(check(&world, "soon", 19), Ok(()));
    assert_eq!(
        check(&world, "soon", 20),
        Err(JoinRefusal::InviteNoLongerValid)
    );
}

#[test]
fn an_invite_never_created_is_unknown() {
    let creator = Member::new(1);
    let world = World::new(&creator);
    assert_eq!(check(&world, "nope", 10), Err(JoinRefusal::InviteUnknown));
}
