//! The device-key act builders refuse what cannot be written from the
//! held set (ADR 0022 rules 4, 5, 7 and 10), before anything is signed.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;

use super::{last_seq_of, readmission};
use crate::apply::Held;
use crate::error::GovernanceRuntimeError;
use crate::op_log::VerifiedSet;

#[test]
fn a_readmit_of_a_member_with_no_device_set_is_refused() {
    let state = GovernanceState::default();
    let set = VerifiedSet::default();
    let author = PseudonymKey([1; 32]);
    let on = Held {
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
    let on = Held {
        state: &state,
        set: &set,
        author: &author,
    };
    assert_eq!(last_seq_of(&on, [3; 32]), 0);
}
