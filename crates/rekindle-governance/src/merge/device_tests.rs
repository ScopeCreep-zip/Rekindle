//! The device phase and the device rows (ADR 0022 rules 6–8, plan E3.2
//! S7d-keys.1).

use rekindle_types::governance::{
    AdmissionMode, DeviceEndorsement, DeviceReadmit, DeviceRetired, GovernanceEntry,
};
use rekindle_types::id::PseudonymKey;

use crate::fixture::{cert, op};
use crate::state::GovernanceState;
use crate::validate::validate_write;

fn retired() -> GovernanceEntry {
    GovernanceEntry::DeviceRetired(DeviceRetired {
        device_key: [9; 32],
        incarnation: 1,
        last_seq: 0,
        retirement_sig: vec![0; 64],
    })
}

fn readmit(member: &PseudonymKey) -> GovernanceEntry {
    GovernanceEntry::DeviceReadmit(DeviceReadmit {
        member: member.clone(),
        member_record: "VLD0:m".into(),
        new_key: [8; 32],
        incarnation: 2,
        fork_evidence: [1; 32],
    })
}

/// A device op resolves before a power op in its own causal past.
#[test]
fn device_ops_resolve_before_power_ops() {
    let creator = PseudonymKey([1; 32]);
    let c = cert(&creator, AdmissionMode::Open);
    let power = op(
        &creator,
        0,
        &[],
        1,
        GovernanceEntry::MemberApproved {
            target: PseudonymKey([2; 32]),
        },
    );
    let device = op(&creator, 1, &[&power], 2, retired());
    let out = super::merge_outcome(&c, &[power.clone(), device.clone()]);
    assert_eq!(out.applied, vec![device.hash(), power.hash()]);
}

/// Endorsements and retirements are valid from any carrier: a banned
/// author's included.
#[test]
fn a_retirement_is_never_refused_on_its_carrier_s_standing() {
    let banned = PseudonymKey([3; 32]);
    let mut state = GovernanceState::default();
    state.bans.insert(banned.clone());
    assert!(validate_write(&banned, &retired(), &state));
}

/// A readmit is the member's own or an outranking kicker's.
#[test]
fn a_readmit_of_another_member_needs_the_kick_rank() {
    let writer = PseudonymKey([4; 32]);
    let state = GovernanceState::default();
    assert!(validate_write(&writer, &readmit(&writer), &state));
    assert!(!validate_write(
        &writer,
        &readmit(&PseudonymKey([5; 32])),
        &state
    ));
    let mut answered_none = readmit(&writer);
    if let GovernanceEntry::DeviceReadmit(r) = &mut answered_none {
        r.fork_evidence = [0; 32];
    }
    assert!(
        !validate_write(&writer, &answered_none, &state),
        "answers no fork"
    );
}

/// A malformed endorsement is refused whoever carries it, the creator too.
#[test]
fn a_malformed_endorsement_is_refused_even_from_the_creator() {
    let creator = PseudonymKey([1; 32]);
    let mut state = GovernanceState::default();
    state.creators.insert(creator.clone());
    let e = DeviceEndorsement {
        device_key: [9; 32],
        incarnation: 1,
        extends_set: [0; 32],
        extends_elements: Vec::new(),
        device_record: "VLD0:d".into(),
        device_record_owner: "own".into(),
        fork_evidence: [0; 32],
        endorsement_sig: vec![0; 64],
    };
    assert!(!validate_write(
        &creator,
        &GovernanceEntry::DeviceKey(e.clone()),
        &state
    ));
    let joined = |first: DeviceEndorsement| GovernanceEntry::MemberJoined {
        member_record: "VLD0:m".into(),
        member_record_owner: "own".into(),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: Some(first),
    };
    assert!(!validate_write(&creator, &joined(e.clone()), &state));
    let mut good = e;
    good.extends_set = crate::device::device_set_hash(&[]);
    assert!(validate_write(
        &creator,
        &GovernanceEntry::DeviceKey(good.clone()),
        &state
    ));
    assert!(validate_write(&creator, &joined(good), &state));
}
