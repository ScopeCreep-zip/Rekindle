//! The device writers produce elements every reader rules effective
//! (ADR 0022 rules 2, 4, 7, 10; plan E3.2 S7d-keys K5).

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{
    AdmissionMode, GenesisCert, GovernanceEntry, GovernanceOp, OpHash,
};
use rekindle_types::id::PseudonymKey;

use super::{endorse, readmit, retire, DeviceRecord, DeviceWriteError};
use crate::device::fork_evidence_hash;
use crate::device_judge::{judge_op, DeviceView, KeyJudgement, KeyRefusal};

const ANCHOR: &str = "VLD0:anchor";
const RECORD: &str = "VLD0:member";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn pk(seed: u8) -> PseudonymKey {
    PseudonymKey(key(seed).verifying_key().to_bytes())
}

fn device(seed: u8) -> [u8; 32] {
    key(seed).verifying_key().to_bytes()
}

fn record(seed: u8) -> DeviceRecord {
    DeviceRecord {
        record: format!("VLD0:device-{seed}"),
        owner: format!("VLD0:owner-{seed}"),
    }
}

fn op(author: u8, device_key: [u8; 32], seq: u64, body: GovernanceEntry) -> GovernanceOp {
    GovernanceOp {
        author: pk(author),
        seq,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body,
        sig: [0; 64],
        device_key,
    }
}

fn plain(author: u8, device_key: [u8; 32], seq: u64) -> GovernanceOp {
    op(
        author,
        device_key,
        seq,
        GovernanceEntry::MemberLeft { heads: Vec::new() },
    )
}

/// `author`'s `MemberJoined`, seq 0 of its first device `first`, carrying
/// that device's endorsement (S7d-keys.6).
fn joined(author: u8, first: u8) -> GovernanceOp {
    let e = endorse(
        &key(author),
        ANCHOR,
        RECORD,
        None,
        device(first),
        record(first),
        [0; 32],
    )
    .unwrap();
    op(
        author,
        device(first),
        0,
        GovernanceEntry::MemberJoined {
            member_record: RECORD.into(),
            member_record_owner: "VLD0:owner".into(),
            invite_proof: None,
            owner_proof: vec![0; 64],
            first_device: Some(e),
        },
    )
}

/// Judge `o` at `view` (its deps), then add it to `view` as a reader does.
fn judge_then_hold(view: &mut DeviceView, o: &GovernanceOp) -> KeyJudgement {
    let verdict = judge_op(ANCHOR, view, o);
    view.absorb(ANCHOR, o);
    verdict
}

/// Member 1 joins on device 11, adds device 12 from device 11's chain,
/// and both devices write.
fn honest_member() -> (DeviceView, Vec<KeyJudgement>) {
    let mut view = DeviceView::default();
    let mut verdicts = vec![judge_then_hold(&mut view, &joined(1, 11))];
    for seq in 1..=2 {
        verdicts.push(judge_then_hold(&mut view, &plain(1, device(11), seq)));
    }
    let held = view.member(&pk(1)).cloned();
    let second = endorse(
        &key(1),
        ANCHOR,
        RECORD,
        held.as_ref(),
        device(12),
        record(12),
        [0; 32],
    )
    .unwrap();
    let carrier = op(1, device(11), 3, GovernanceEntry::DeviceKey(second.clone()));
    verdicts.push(judge_then_hold(&mut view, &carrier));
    // Device 12's own chain starts with an op carrying its endorsement.
    let first_of_12 = op(1, device(12), 0, GovernanceEntry::DeviceKey(second));
    verdicts.push(judge_then_hold(&mut view, &first_of_12));
    verdicts.push(judge_then_hold(&mut view, &plain(1, device(12), 1)));
    verdicts.push(judge_then_hold(&mut view, &plain(1, device(11), 4)));
    (view, verdicts)
}

#[test]
fn an_honest_members_ops_all_judge_effective() {
    let (_, verdicts) = honest_member();
    assert_eq!(verdicts.len(), 7);
    assert!(
        verdicts.iter().all(|v| *v == KeyJudgement::Effective),
        "{verdicts:?}"
    );
}

#[test]
fn the_creators_genesis_ops_judge_effective_through_the_cert() {
    let creator_device = endorse(
        &key(1),
        ANCHOR,
        RECORD,
        None,
        device(11),
        record(11),
        [0; 32],
    )
    .unwrap();
    let cert = GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: pk(1),
        creator_member_record: RECORD.into(),
        creator_member_record_owner: "VLD0:owner".into(),
        created_at: 0,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: Some(creator_device),
    };
    let mut view = DeviceView::default();
    view.seed_genesis(ANCHOR, &cert);
    for seq in 0..3 {
        assert_eq!(
            judge_then_hold(&mut view, &plain(1, device(11), seq)),
            KeyJudgement::Effective
        );
    }
}

#[test]
fn a_retired_devices_later_op_judges_refused() {
    let (mut view, _) = honest_member();
    let held = view.member(&pk(1)).cloned().unwrap();
    // Device 11 wrote seqs 0..=4; device 12 carries its retirement at 4.
    let retired = retire(&key(1), ANCHOR, RECORD, &held, device(11), 4).unwrap();
    let carrier = op(1, device(12), 2, GovernanceEntry::DeviceRetired(retired));
    assert_eq!(
        judge_then_hold(&mut view, &carrier),
        KeyJudgement::Effective
    );
    assert_eq!(
        judge_op(ANCHOR, &view, &plain(1, device(11), 5)),
        KeyJudgement::Refused(KeyRefusal::Retired)
    );
    // Its ops at or below `lastSeq` still count, and device 12 writes on.
    assert_eq!(
        judge_op(ANCHOR, &view, &plain(1, device(11), 4)),
        KeyJudgement::Effective
    );
    assert_eq!(
        judge_op(ANCHOR, &view, &plain(1, device(12), 3)),
        KeyJudgement::Effective
    );
}

#[test]
fn a_readmit_restores_the_replacement_key() {
    // Member 1's sole device 11 forks at seq 2; moderator 2 is on device 21.
    let mut view = DeviceView::default();
    judge_then_hold(&mut view, &joined(1, 11));
    judge_then_hold(&mut view, &joined(2, 21));
    view.fork(&pk(1), device(11), 2);
    let evidence = fork_evidence_hash(ANCHOR, &device(11), 2, [OpHash([1; 32]), OpHash([2; 32])]);
    let held = view.member(&pk(1)).cloned();
    let replacement = endorse(
        &key(1),
        ANCHOR,
        RECORD,
        held.as_ref(),
        device(13),
        record(13),
        evidence,
    )
    .unwrap();
    let first_of_13 = op(
        1,
        device(13),
        0,
        GovernanceEntry::DeviceKey(replacement.clone()),
    );
    assert_eq!(
        judge_op(ANCHOR, &view, &first_of_13),
        KeyJudgement::Refused(KeyRefusal::NeedsReadmit)
    );
    let body = readmit(&pk(1), RECORD, &replacement).unwrap();
    let co_signed = op(2, device(21), 1, GovernanceEntry::DeviceReadmit(body));
    assert_eq!(
        judge_then_hold(&mut view, &co_signed),
        KeyJudgement::Effective
    );
    assert_eq!(
        judge_op(ANCHOR, &view, &first_of_13),
        KeyJudgement::Effective
    );
}

#[test]
fn the_writers_refuse_what_no_reader_would_take() {
    let (view, _) = honest_member();
    let held = view.member(&pk(1)).unwrap();
    assert_eq!(
        retire(&key(1), ANCHOR, RECORD, held, device(99), 0),
        Err(DeviceWriteError::NotEndorsed)
    );
    let e = endorse(
        &key(1),
        ANCHOR,
        RECORD,
        Some(held),
        device(14),
        record(14),
        [0; 32],
    )
    .unwrap();
    assert_eq!(e.incarnation, 3, "above both endorsed incarnations");
    assert_eq!(
        readmit(&pk(1), RECORD, &e),
        Err(DeviceWriteError::AnswersNoFork)
    );
}
