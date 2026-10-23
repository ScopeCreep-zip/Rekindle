//! Our ops chain: seq 0 has no prev and no deps; the next cites it as prev
//! and dep; both verify as a reader verifies them, and `causal_past`
//! replays a frontier's past only.

use super::*;
use rekindle_governance::fixture::meta;
use rekindle_types::governance::AdmissionMode;

fn cert(key: &SigningKey) -> GenesisCert {
    GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey(key.verifying_key().to_bytes()),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
    }
}

#[test]
fn our_ops_chain_verify_and_replay_by_frontier() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = cert(&key);
    let state = GovernanceState::default();
    let empty = VerifiedSet::default();
    let (first, first_bytes) = build_op("A", &cert, &state, &empty, &key, meta("a"), 5).unwrap();
    assert_eq!(
        (first.op().seq, first.op().prev, first.lamport()),
        (0, None, 1)
    );
    let record = author_record(&cert, &state, first.author()).unwrap();
    let held_first = held(&first, record.clone(), first_bytes, OpFate::Applied);
    let set = verify_held("A", std::slice::from_ref(&held_first));
    assert_eq!(set.ops, vec![first.clone()]);
    let (second, second_bytes) = build_op("A", &cert, &state, &set, &key, meta("b"), 6).unwrap();
    assert_eq!(second.op().seq, 1);
    assert_eq!(second.op().prev, Some(first.hash()));
    assert_eq!(second.op().deps, vec![first.op_ref(&record)]);
    assert_eq!(second.lamport(), 2);
    let both = [
        held_first,
        held(&second, record.clone(), second_bytes, OpFate::Applied),
    ];
    let set = verify_held("A", &both);
    assert_eq!(set.ops.len(), 2);
    assert!(set.pending.is_empty() && set.quarantined.is_empty());
    // Under another anchor nothing verifies.
    assert_eq!(verify_held("B", &both).quarantined.len(), 2);
    let past = causal_past(&set.ops, &[first.op_ref(&record)]);
    assert_eq!(past, vec![first]);
}

#[test]
fn a_held_op_whose_dep_is_absent_is_pending_and_its_dep_is_missing() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = cert(&key);
    let state = GovernanceState::default();
    let (first, first_bytes) = build_op(
        "A",
        &cert,
        &state,
        &VerifiedSet::default(),
        &key,
        meta("a"),
        5,
    )
    .unwrap();
    let set = verify_held(
        "A",
        &[held(&first, "m".into(), first_bytes, OpFate::Applied)],
    );
    let (second, bytes) = build_op("A", &cert, &state, &set, &key, meta("b"), 6).unwrap();
    let alone = verify_held("A", &[held(&second, "m".into(), bytes, OpFate::Applied)]);
    assert_eq!(alone.pending, vec![second.hash()]);
    assert_eq!(alone.missing.len(), 1);
    assert_eq!(alone.missing[0].op_hash, first.hash());
}
