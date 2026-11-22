//! A re-join and the person's other devices (ADR 0022 amended 2026-11-22,
//! lead decision 63): the re-joining device's endorsement, signed for the
//! new `M(p)` its `MemberJoined` names, is taken; a key endorsed only under
//! the ended membership binds no current record; a fresh key endorsed for
//! the new record by a re-enrolling device is taken.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use super::enrol_tests::{enrol_op, held, joined_view, op, p, record, ANCHOR, M_P};
use super::DeviceView;
use crate::device_writer::endorse;

/// The re-join's new member record.
const M_P2: &str = "VLD0:member-2";

fn pk() -> PseudonymKey {
    PseudonymKey(p(1).verifying_key().to_bytes())
}

/// `joined_view` plus device `[11; 32]` on `M(p,d)` under the first
/// membership, then a re-join on `M_P2` whose first device is `[12; 32]`.
fn rejoined_view() -> DeviceView {
    let mut view = joined_view();
    let second = endorse(
        &p(1),
        ANCHOR,
        M_P,
        Some(held(&view)),
        [11; 32],
        record("VLD0:device-11"),
        [0; 32],
    )
    .unwrap();
    view.absorb(ANCHOR, &enrol_op(second));
    let first = endorse(&p(1), ANCHOR, M_P2, None, [12; 32], record(M_P2), [0; 32]).unwrap();
    view.absorb(
        ANCHOR,
        &op(
            9,
            12,
            GovernanceEntry::MemberJoined {
                member_record: M_P2.into(),
                member_record_owner: "VLD0:owner-2".into(),
                invite_proof: None,
                owner_proof: vec![0; 64],
                first_device: Some(first),
            },
        ),
    );
    view
}

#[test]
fn a_rejoins_first_device_is_endorsed_for_the_new_record() {
    let view = rejoined_view();
    assert!(held(&view)
        .endorsement_for(&pk(), ANCHOR, M_P2, &[12; 32])
        .is_some());
}

#[test]
fn a_key_endorsed_under_the_ended_membership_binds_no_current_record() {
    let view = rejoined_view();
    let md = held(&view);
    assert!(md.endorsement_for(&pk(), ANCHOR, M_P, &[11; 32]).is_some());
    assert!(md.endorsement_for(&pk(), ANCHOR, M_P2, &[11; 32]).is_none());
}

#[test]
fn a_fresh_key_enrolled_for_the_new_record_is_taken() {
    let mut view = rejoined_view();
    let fresh = endorse(
        &p(1),
        ANCHOR,
        M_P2,
        Some(held(&view)),
        [13; 32],
        record("VLD0:device-13"),
        [0; 32],
    )
    .unwrap();
    view.absorb(ANCHOR, &enrol_op(fresh));
    assert!(held(&view)
        .endorsement_for(&pk(), ANCHOR, M_P2, &[13; 32])
        .is_some());
}
