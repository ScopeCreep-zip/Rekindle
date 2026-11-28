//! The device-key act builders refuse what cannot be written from the
//! held set (ADR 0022 rules 4, 5, 7 and 10), before anything is signed.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;

use super::{answered_fork, last_seq_of, readmission};
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

/// An enrolment after a retirement names the retirement (ADR 0022 rule 7,
/// amended 2026-11-23), and a key endorsed before it (the member's first
/// device, its join's) co-signs the readmit that makes the new key
/// effective: for a joined member, whose set its join seeds, and for the
/// creator, whose set the cert seeds (S7d-keys.5).
#[test]
fn enrol_after_retire_names_the_retirement_and_a_pre_fork_device_readmits_it() {
    enrol_after_retire_readmits(false);
}

#[test]
fn the_creators_enrol_after_retire_names_the_retirement_and_readmits() {
    enrol_after_retire_readmits(true);
}

fn enrol_after_retire_readmits(creator_subject: bool) {
    use rekindle_governance::device::retirement_evidence_hash;
    use rekindle_governance::device_writer::{endorse, readmit, retire, DeviceRecord};
    use rekindle_governance::merge;
    use rekindle_secrets::ed25519_dalek::SigningKey;
    use rekindle_types::governance::GovernanceEntry;

    use crate::op_log::Deps;
    use crate::test_member::Member;
    use crate::test_world::{World, ANCHOR};
    let creator = Member::new(1);
    let member = Member::new(2);
    let mut world = World::new(&creator);
    world.join(&member, &creator, "code");
    let subject = if creator_subject { &creator } else { &member };
    let author = subject.author();
    let record = subject.record();
    let on = |w: &World| {
        answered_fork(
            &BuiltOn {
                state: &w.state,
                set: &w.set,
                author: &author,
            },
            &record,
        )
    };
    assert_eq!(on(&world), [0; 32], "no fork event yet");
    // A device record as Veilid names one, with its owner's key text.
    let rec = |seed: u8| {
        let m = Member::new(seed);
        DeviceRecord {
            record: m.record(),
            owner: m.owner_text(),
        }
    };
    let set = |w: &World| w.state.devices.member(&author).cloned();
    // Each new key's endorsement is its own seq-0 op, as `enrol` writes it.
    let device = |seed: u8| Member {
        pseudonym: subject.pseudonym.clone(),
        owner: subject.owner.clone(),
        device: SigningKey::from_bytes(&[seed; 32]),
    };
    let (second, third) = (device(42), device(43));
    let k2_pub = second.device.verifying_key().to_bytes();
    let e2 = endorse(
        &subject.pseudonym,
        ANCHOR,
        &record,
        set(&world).as_ref(),
        k2_pub,
        rec(52),
        [0; 32],
    )
    .unwrap();
    world.author(&second, GovernanceEntry::DeviceKey(e2), Deps::Frontier);
    let held = set(&world).unwrap();
    let retired = retire(&subject.pseudonym, ANCHOR, &record, &held, k2_pub, 0).unwrap();
    let evidence = retirement_evidence_hash(ANCHOR, &record, &retired);
    world.author(
        subject,
        GovernanceEntry::DeviceRetired(retired),
        Deps::Frontier,
    );
    assert_eq!(on(&world), evidence, "the enrolment names the retirement");
    let k3_pub = third.device.verifying_key().to_bytes();
    let e3 = endorse(
        &subject.pseudonym,
        ANCHOR,
        &record,
        set(&world).as_ref(),
        k3_pub,
        rec(53),
        evidence,
    )
    .unwrap();
    world.author(
        &third,
        GovernanceEntry::DeviceKey(e3.clone()),
        Deps::Frontier,
    );
    let body = readmit(&author, &record, &e3).unwrap();
    let r = world.author(
        subject,
        GovernanceEntry::DeviceReadmit(body),
        Deps::Frontier,
    );
    // The joined member holds no role, and a leave needs none; the
    // creator holds every permission.
    let last = if creator_subject {
        rekindle_governance::fixture::meta("after")
    } else {
        GovernanceEntry::MemberLeft { heads: Vec::new() }
    };
    let after = world.author(&third, last, Deps::Frontier);
    let out = merge::merge_outcome(&world.cert, &world.set.ops);
    assert!(out.applied.contains(&r.hash()), "{out:?}");
    assert!(out.applied.contains(&after.hash()), "{out:?}");
}
