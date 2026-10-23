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

#[test]
fn a_refused_op_is_quarantined_and_a_verified_op_carries_its_lamport() {
    let creator = SigningKey::from_bytes(&[7; 32]);
    let stranger = SigningKey::from_bytes(&[9; 32]);
    let cert = cert(&creator);
    let state = GovernanceState::default();
    let empty = VerifiedSet::default();
    let (ours, ours_bytes) = build_op("A", &cert, &state, &empty, &creator, meta("a"), 5).unwrap();
    // A non-member's write verifies (signature, chain) but the merge refuses it.
    let (theirs, theirs_bytes) =
        build_op("A", &cert, &state, &empty, &stranger, meta("b"), 5).unwrap();
    let set = verify_held(
        "A",
        &[
            held(&ours, "m".into(), ours_bytes, OpFate::PendingDeps),
            held(&theirs, "s".into(), theirs_bytes, OpFate::PendingDeps),
        ],
    );
    let outcome = merge::merge_outcome(&cert, &set.ops);
    assert!(outcome.refused.contains(&theirs.hash()));
    let verdicts = fates(&set, &outcome);
    let of = |hash| *verdicts.iter().find(|v| v.hash == hash).unwrap();
    assert_eq!(
        of(ours.hash()),
        OpVerdict {
            hash: ours.hash(),
            fate: OpFate::Applied,
            lamport: Some(1),
        }
    );
    assert_eq!(of(theirs.hash()).fate, OpFate::Quarantined);
    assert_eq!(of(theirs.hash()).lamport, Some(1));
}

fn joined(key: &SigningKey, code_hash: Option<&str>) -> GovernanceOp {
    GovernanceOp {
        author: PseudonymKey(key.verifying_key().to_bytes()),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 1,
        body: GovernanceEntry::MemberJoined {
            member_record: "VLD0:j".into(),
            member_record_owner: "o".into(),
            invite_proof: code_hash.map(|c| rekindle_types::governance::InviteProof {
                code_hash: c.into(),
                sig: vec![0; 64],
            }),
        },
        sig: [0; 64],
    }
}

#[test]
fn a_gossiped_join_is_stored_only_against_a_live_invite_and_within_bounds() {
    let joiner = SigningKey::from_bytes(&[3; 32]);
    let mut state = GovernanceState::default();
    assert!(!receive_bounded(&joined(&joiner, None), &state));
    assert!(!receive_bounded(&joined(&joiner, Some("live")), &state));
    state.invites.insert(
        [1; 16],
        rekindle_governance::state::InviteState {
            code_hash: "live".into(),
            max_uses: 1,
            expires_at: None,
            secrets_record_key: "VLD0:s".into(),
            created: rekindle_governance::op::OpStamp::MIN,
            creator_pseudonym: PseudonymKey([1; 32]),
        },
    );
    assert!(receive_bounded(&joined(&joiner, Some("live")), &state));
    assert!(!receive_bounded(&joined(&joiner, Some("other")), &state));
    let mut oob = joined(&joiner, Some("live"));
    oob.seq = 1;
    oob.body = GovernanceEntry::CommunityMeta {
        name: None,
        description: Some("x".repeat(rekindle_types::key_format::MAX_DESCRIPTION_LEN + 1)),
        icon_hash: None,
        banner_hash: None,
    };
    assert!(!receive_bounded(&oob, &state));
}
