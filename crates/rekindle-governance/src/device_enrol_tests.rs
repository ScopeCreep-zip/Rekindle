//! A second device enrolling (ADR 0022 rules 1–3, plan E3.2 K5h): its
//! `DeviceKey` op, seq 0 of the new key's chain, carries `p`'s endorsement
//! of the key for its own record `M(p,d)`, built by the production writer
//! from the member's set as the device holds it. Readers judge it.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{DeviceEndorsement, GovernanceEntry, GovernanceOp};
use rekindle_types::id::PseudonymKey;

use super::{judge_op, DeviceView, KeyJudgement, KeyPending, KeyRefusal, MemberDevices};
use crate::device_writer::{endorse, DeviceRecord};

const ANCHOR: &str = "VLD0:anchor";
/// `M(p)`, the first device's record (rule 3).
const M_P: &str = "VLD0:member";

fn p(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn record(name: &str) -> DeviceRecord {
    DeviceRecord {
        record: name.to_owned(),
        owner: format!("{name}-owner"),
    }
}

fn op(seq: u64, device_key: u8, body: GovernanceEntry) -> GovernanceOp {
    GovernanceOp {
        author: PseudonymKey(p(1).verifying_key().to_bytes()),
        seq,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body,
        sig: [0; 64],
        device_key: [device_key; 32],
    }
}

/// Member `p(1)` joined with first device `[10; 32]`, whose record is `M(p)`.
fn joined_view() -> DeviceView {
    let first = endorse(&p(1), ANCHOR, M_P, None, [10; 32], record(M_P), [0; 32]).unwrap();
    let mut view = DeviceView::default();
    view.absorb(
        ANCHOR,
        &op(
            0,
            10,
            GovernanceEntry::MemberJoined {
                member_record: M_P.into(),
                member_record_owner: "VLD0:owner".into(),
                invite_proof: None,
                owner_proof: vec![0; 64],
                first_device: Some(first),
            },
        ),
    );
    view
}

fn held(view: &DeviceView) -> &MemberDevices {
    view.member(&PseudonymKey(p(1).verifying_key().to_bytes()))
        .unwrap()
}

/// `signer`'s endorsement of `[key; 32]` for `rec`, extending the set held.
fn enrolment(view: &DeviceView, signer: u8, key: u8, rec: &str) -> DeviceEndorsement {
    endorse(
        &p(signer),
        ANCHOR,
        M_P,
        Some(held(view)),
        [key; 32],
        record(rec),
        [0; 32],
    )
    .unwrap()
}

/// The new key's own seq-0 op carrying its endorsement.
fn enrol_op(e: DeviceEndorsement) -> GovernanceOp {
    let key = e.device_key[0];
    op(0, key, GovernanceEntry::DeviceKey(e))
}

#[test]
fn a_second_device_endorsed_by_p_for_its_own_record_is_effective() {
    let mut view = joined_view();
    let carrier = enrol_op(enrolment(&view, 1, 11, "VLD0:device-11"));
    assert_eq!(judge_op(ANCHOR, &view, &carrier), KeyJudgement::Effective);
    // Harvested, its later ops are effective too, and the first device stays.
    view.absorb(ANCHOR, &carrier);
    assert_eq!(
        judge_op(
            ANCHOR,
            &view,
            &op(1, 11, GovernanceEntry::MemberLeft { heads: vec![] })
        ),
        KeyJudgement::Effective
    );
    assert_eq!(
        judge_op(
            ANCHOR,
            &view,
            &op(1, 10, GovernanceEntry::MemberLeft { heads: vec![] })
        ),
        KeyJudgement::Effective
    );
}

#[test]
fn rule_1_an_endorsement_not_signed_by_p_here_is_never_taken() {
    let view = joined_view();
    // Signed by another key than the op's author `p`.
    let forged = enrol_op(enrolment(&view, 9, 11, "VLD0:device-11"));
    assert_eq!(
        judge_op(ANCHOR, &view, &forged),
        KeyJudgement::Pending(KeyPending::Unendorsed)
    );
    // Signed by `p`, but for another community (rule 10's context).
    let mut elsewhere = endorse(
        &p(1),
        "VLD0:other",
        M_P,
        Some(held(&view)),
        [11; 32],
        record("VLD0:d"),
        [0; 32],
    )
    .unwrap();
    elsewhere.device_record = "VLD0:d".into();
    assert_eq!(
        judge_op(ANCHOR, &view, &enrol_op(elsewhere)),
        KeyJudgement::Pending(KeyPending::Unendorsed)
    );
}

#[test]
fn rule_2_an_incarnation_not_above_the_extended_set_is_malformed() {
    let view = joined_view();
    let mut stale = enrolment(&view, 1, 11, "VLD0:device-11");
    stale.incarnation = 1;
    assert!(!crate::device::endorsement_well_formed(&stale));
    assert!(crate::device::endorsement_well_formed(&enrolment(
        &view, 1, 11, "VLD0:d"
    )));
}

#[test]
fn rule_3_a_record_bound_by_another_key_is_refused_in_any_order() {
    let mut view = joined_view();
    // A second key naming `M(p)`, the first device's record.
    let on_m_p = enrol_op(enrolment(&view, 1, 11, M_P));
    assert_eq!(
        judge_op(ANCHOR, &view, &on_m_p),
        KeyJudgement::Refused(KeyRefusal::RecordTaken)
    );
    // A third key naming the second's record: the earlier binding keeps it.
    let second = enrol_op(enrolment(&view, 1, 11, "VLD0:device-11"));
    view.absorb(ANCHOR, &second);
    let third = enrol_op(enrolment(&view, 1, 12, "VLD0:device-11"));
    assert_eq!(
        judge_op(ANCHOR, &view, &third),
        KeyJudgement::Refused(KeyRefusal::RecordTaken)
    );
    // The same set absorbed in the other order judges the same.
    let mut reversed = joined_view();
    reversed.absorb(ANCHOR, &third);
    reversed.absorb(ANCHOR, &second);
    assert_eq!(
        judge_op(ANCHOR, &reversed, &second),
        KeyJudgement::Effective
    );
    assert_eq!(
        judge_op(ANCHOR, &reversed, &third),
        KeyJudgement::Refused(KeyRefusal::RecordTaken)
    );
}

#[test]
fn rule_3_an_endorsement_counts_for_its_own_key_only_at_seq_zero() {
    let view = joined_view();
    let e = enrolment(&view, 1, 11, "VLD0:device-11");
    let late = op(3, 11, GovernanceEntry::DeviceKey(e));
    assert_eq!(
        judge_op(ANCHOR, &view, &late),
        KeyJudgement::Pending(KeyPending::Unendorsed)
    );
}
