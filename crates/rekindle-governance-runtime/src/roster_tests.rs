use super::is_admitted;
use rekindle_governance::state::{AdmissionDecision, GovernanceState};
use rekindle_types::governance::AdmissionMode;
use rekindle_types::id::PseudonymKey;

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}
fn hex_of(byte: u8) -> String {
    hex::encode([byte; 32])
}

#[test]
fn an_approved_member_is_admitted() {
    let mut state = GovernanceState::default();
    state.admitted.insert(
        key(1),
        AdmissionDecision {
            approved: true,
            stamp: rekindle_governance::op::OpStamp::MIN,
        },
    );
    assert!(is_admitted(&state, &hex_of(1)));
}

/// A rejected member holds a slot — `o_cnt: 0` cannot stop them —
/// but is not counted. This is the whole enforcement point.
#[test]
fn a_rejected_member_is_not_admitted() {
    let mut state = GovernanceState::default();
    state.admitted.insert(
        key(2),
        AdmissionDecision {
            approved: false,
            stamp: rekindle_governance::op::OpStamp::MIN,
        },
    );
    assert!(!is_admitted(&state, &hex_of(2)));
}

#[test]
fn an_undecided_member_is_not_admitted() {
    assert!(!is_admitted(&GovernanceState::default(), &hex_of(3)));
}

/// The creator wrote the genesis entries, so requiring an approval
/// they could only give themselves would lock them out of their own
/// community on the first merge.
#[test]
fn the_creator_is_admitted_without_a_decision() {
    let state = GovernanceState {
        creators: std::collections::BTreeSet::from([key(9)]),
        ..Default::default()
    };
    assert!(state.admitted.is_empty());
    assert!(is_admitted(&state, &hex_of(9)));
}

/// Default is `Open` — every community that predates the admission
/// entries must keep admitting everyone, or the roster would empty
/// itself on upgrade.
#[test]
fn absent_admission_policy_means_open() {
    let state = GovernanceState::default();
    assert_eq!(state.effective_admission_mode(), AdmissionMode::Open);
}
