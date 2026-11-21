//! Device keys judged at causal deps (ADR 0022 rules 2, 6, 7; plan E3.2
//! S7d-keys.2).

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    DeviceEndorsement, DeviceReadmit, DeviceRetired, DeviceSetElement, DeviceSetElementKind,
    GovernanceEntry, GovernanceOp,
};
use rekindle_types::id::PseudonymKey;

use super::{judge_op, DeviceView, KeyJudgement, KeyPending, KeyRefusal};
use crate::device::{device_set_hash, endorsement_signing_bytes, retirement_signing_bytes};

const ANCHOR: &str = "VLD0:community";
const RECORD: &str = "VLD0:member";

fn p(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn pk(seed: u8) -> PseudonymKey {
    PseudonymKey(p(seed).verifying_key().to_bytes())
}

fn element(kind: DeviceSetElementKind, key: u8, incarnation: u64) -> DeviceSetElement {
    DeviceSetElement::new(kind, [key; 32], incarnation)
}

/// `p(seed)`'s signed endorsement of `[key; 32]` at `incarnation`.
fn endorse(
    seed: u8,
    key: u8,
    incarnation: u64,
    mut extends: Vec<DeviceSetElement>,
    fork_evidence: [u8; 32],
) -> DeviceEndorsement {
    extends.sort_unstable();
    let mut e = DeviceEndorsement {
        device_key: [key; 32],
        incarnation,
        extends_set: device_set_hash(&extends),
        extends_elements: extends,
        device_record: format!("VLD0:device-{key}"),
        device_record_owner: format!("VLD0:owner-{key}"),
        fork_evidence,
        endorsement_sig: Vec::new(),
    };
    let bytes = endorsement_signing_bytes(ANCHOR, RECORD, &e);
    e.endorsement_sig = p(seed).sign(&bytes).to_bytes().to_vec();
    e
}

fn op(author: u8, device_key: u8, seq: u64, body: GovernanceEntry) -> GovernanceOp {
    GovernanceOp {
        author: pk(author),
        seq,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body,
        sig: [0; 64],
        device_key: [device_key; 32],
    }
}

fn joined(author: u8, first: Option<DeviceEndorsement>) -> GovernanceOp {
    let key = first.as_ref().map_or(0, |e| e.device_key[0]);
    op(
        author,
        key,
        0,
        GovernanceEntry::MemberJoined {
            member_record: RECORD.into(),
            member_record_owner: "VLD0:owner".into(),
            invite_proof: None,
            owner_proof: vec![0; 64],
            first_device: first,
        },
    )
}

fn rename(author: u8, device_key: u8, seq: u64) -> GovernanceOp {
    op(
        author,
        device_key,
        seq,
        GovernanceEntry::MemberLeft { heads: Vec::new() },
    )
}

/// A view where `author` joined with first device `[first; 32]`.
fn view_with(author: u8, first: u8) -> DeviceView {
    let mut view = DeviceView::default();
    view.absorb(
        ANCHOR,
        &joined(author, Some(endorse(author, first, 1, vec![], [0; 32]))),
    );
    view
}

#[test]
fn an_endorsement_is_pending_until_its_extended_set_is_held() {
    let lacking = element(DeviceSetElementKind::Endorsement, 7, 1);
    let mut view = view_with(1, 10);
    let second = endorse(1, 11, 2, vec![lacking], [0; 32]);
    view.absorb(ANCHOR, &op(1, 10, 1, GovernanceEntry::DeviceKey(second)));
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 11, 0)),
        KeyJudgement::Pending(KeyPending::ExtendedSet(lacking))
    );
    // Once the extended element arrives, the key is effective.
    let seventh = endorse(1, 7, 1, vec![], [0; 32]);
    view.absorb(ANCHOR, &op(1, 10, 2, GovernanceEntry::DeviceKey(seventh)));
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 11, 0)),
        KeyJudgement::Effective
    );
}

#[test]
fn an_op_of_a_key_whose_endorsement_has_not_arrived_is_pending_not_quarantined() {
    let view = view_with(1, 10);
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 12, 0)),
        KeyJudgement::Pending(KeyPending::Unendorsed)
    );
    assert_eq!(
        judge_op(ANCHOR, &DeviceView::default(), &rename(2, 20, 3)),
        KeyJudgement::Pending(KeyPending::Unendorsed)
    );
}

#[test]
fn a_readmit_is_never_quarantined_by_its_subjects_retirement() {
    // Member 1's only key is retired; moderator 2 re-admits member 1.
    let mut view = view_with(1, 10);
    view.union(&view_with(2, 20));
    let mut retired = DeviceRetired {
        device_key: [10; 32],
        incarnation: 1,
        last_seq: 0,
        retirement_sig: Vec::new(),
    };
    let bytes = retirement_signing_bytes(ANCHOR, RECORD, &retired);
    retired.retirement_sig = p(1).sign(&bytes).to_bytes().to_vec();
    view.absorb(
        ANCHOR,
        &op(1, 10, 1, GovernanceEntry::DeviceRetired(retired)),
    );
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 10, 2)),
        KeyJudgement::Refused(KeyRefusal::Retired)
    );
    let readmit = op(
        2,
        20,
        1,
        GovernanceEntry::DeviceReadmit(DeviceReadmit {
            member: pk(1),
            member_record: RECORD.into(),
            new_key: [13; 32],
            incarnation: 2,
            fork_evidence: [5; 32],
        }),
    );
    assert_eq!(judge_op(ANCHOR, &view, &readmit), KeyJudgement::Effective);
}

#[test]
fn a_seventh_live_device_key_is_inert() {
    let mut view = view_with(1, 10);
    for key in 11..=16 {
        let e = endorse(1, key, u64::from(key), vec![], [0; 32]);
        view.absorb(
            ANCHOR,
            &op(1, 10, u64::from(key), GovernanceEntry::DeviceKey(e)),
        );
    }
    for key in 10..=15 {
        assert_eq!(
            judge_op(ANCHOR, &view, &rename(1, key, 0)),
            KeyJudgement::Effective
        );
    }
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 16, 0)),
        KeyJudgement::Refused(KeyRefusal::OverCap)
    );
    // An endorsement whose own extended set holds six live keys is inert.
    let six = (10..=15)
        .map(|k| {
            element(
                DeviceSetElementKind::Endorsement,
                k,
                if k == 10 { 1 } else { u64::from(k) },
            )
        })
        .collect();
    let e = endorse(1, 17, 20, six, [0; 32]);
    view.absorb(ANCHOR, &op(1, 10, 30, GovernanceEntry::DeviceKey(e)));
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 17, 0)),
        KeyJudgement::Refused(KeyRefusal::Inert)
    );
}

#[test]
fn an_op_signed_by_a_non_effective_key_is_refused() {
    // Answers a fork with no matching readmit.
    let mut view = view_with(1, 10);
    let e = endorse(
        1,
        11,
        2,
        vec![element(DeviceSetElementKind::Endorsement, 10, 1)],
        [5; 32],
    );
    view.absorb(ANCHOR, &op(1, 10, 1, GovernanceEntry::DeviceKey(e)));
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 11, 0)),
        KeyJudgement::Refused(KeyRefusal::NeedsReadmit)
    );
    // Forked at seq 3: its ops from seq 3 are refused, below stay effective.
    view.fork(&pk(1), [10; 32], 3);
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 10, 2)),
        KeyJudgement::Effective
    );
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 10, 3)),
        KeyJudgement::Refused(KeyRefusal::Retired)
    );
    // An endorsement signed by another pseudonym is never taken.
    let forged = endorse(9, 12, 3, vec![], [0; 32]);
    view.absorb(ANCHOR, &op(1, 10, 2, GovernanceEntry::DeviceKey(forged)));
    assert_eq!(
        judge_op(ANCHOR, &view, &rename(1, 12, 0)),
        KeyJudgement::Pending(KeyPending::Unendorsed)
    );
}
