//! The device-key act builders refuse what cannot be written from the
//! held set (ADR 0022 rules 4, 5, 7 and 10), before anything is signed.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;

use super::{last_seq_of, readmission};
use crate::apply::BuiltOn;
use crate::error::GovernanceRuntimeError;
use crate::op_log::VerifiedSet;

#[test]
fn a_readmit_of_a_member_with_no_device_set_is_refused() {
    let state = GovernanceState::default();
    let set = VerifiedSet::default();
    let author = PseudonymKey([1; 32]);
    let on = BuiltOn {
        state: &state,
        set: &set,
        author: &author,
    };
    assert!(matches!(
        readmission(&on, &PseudonymKey([2; 32]), [3; 32]),
        Err(GovernanceRuntimeError::DeviceActRefused(_))
    ));
}

#[test]
fn a_key_with_no_held_op_retires_above_its_seq_zero() {
    let state = GovernanceState::default();
    let set = VerifiedSet::default();
    let author = PseudonymKey([1; 32]);
    let on = BuiltOn {
        state: &state,
        set: &set,
        author: &author,
    };
    assert_eq!(last_seq_of(&on, [3; 32]), 0);
}

/// A `DeviceRetired` built on a set holding our key's ops at seqs 0..=n
/// retires it above n, never below an op in its carrier's causal past
/// (ADR 0022 rule 5); another member's ops and our other keys' never
/// count.
#[test]
fn a_key_with_held_ops_at_seqs_zero_to_n_retires_above_n() {
    use crate::op_log::Deps;
    use crate::test_member::Member;
    use crate::test_world::World;
    let creator = Member::new(1);
    let joiner = Member::new(2);
    let mut world = World::new(&creator);
    world.join(&joiner, &creator, "code");
    let key = creator.device.verifying_key().to_bytes();
    let held = (world.set.ops.iter())
        .filter(|op| op.op().device_key == key)
        .count();
    let n = u64::try_from(held).unwrap() - 1;
    assert!(n >= 5, "the creator's genesis ops and invite");
    let author = creator.author();
    let on = BuiltOn {
        state: &world.state,
        set: &world.set,
        author: &author,
    };
    assert_eq!(last_seq_of(&on, key), n);
    // The joiner's key is not ours: nothing of it is ours to retire.
    let theirs = joiner.device.verifying_key().to_bytes();
    assert_eq!(last_seq_of(&on, theirs), 0);
    // One more op of ours moves the floor with it.
    world.author(
        &creator,
        rekindle_governance::fixture::meta("m"),
        Deps::Frontier,
    );
    let on = BuiltOn {
        state: &world.state,
        set: &world.set,
        author: &author,
    };
    assert_eq!(last_seq_of(&on, key), n + 1);
}

/// Rule 3 at the enrolling writer (plan K5k): the member record is the
/// first device's, a key the set endorses is never endorsed again, and a
/// record another key binds is taken; a fresh key on a fresh record goes.
#[test]
fn enrol_refuses_an_endorsed_key_and_a_taken_record() {
    use rekindle_governance::device_judge::MemberDevices;
    use rekindle_governance::device_writer::{endorse, DeviceRecord};
    use rekindle_secrets::ed25519_dalek::SigningKey;

    use super::refuse_taken;
    let p = SigningKey::from_bytes(&[1; 32]);
    let bound = endorse(
        &p,
        "VLD0:anchor",
        "VLD0:m",
        None,
        [7; 32],
        DeviceRecord {
            record: "VLD0:mpd".into(),
            owner: "o".into(),
        },
        [0; 32],
    )
    .unwrap();
    let mut md = MemberDevices::default();
    md.endorsements.insert((1, [7; 32]), bound);
    let refused = |r: Result<(), GovernanceRuntimeError>| {
        matches!(r, Err(GovernanceRuntimeError::DeviceActRefused(_)))
    };
    assert!(refused(refuse_taken(None, [8; 32], "VLD0:m", "VLD0:m")));
    assert!(refused(refuse_taken(
        Some(&md),
        [7; 32],
        "VLD0:new",
        "VLD0:m"
    )));
    assert!(refused(refuse_taken(
        Some(&md),
        [8; 32],
        "VLD0:mpd",
        "VLD0:m"
    )));
    assert!(refuse_taken(Some(&md), [8; 32], "VLD0:new", "VLD0:m").is_ok());
}
