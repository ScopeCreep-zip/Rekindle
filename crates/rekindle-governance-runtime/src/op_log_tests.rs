//! Our ops chain: seq 0 has no prev and no deps; the next cites it as prev
//! and dep; both verify as a reader verifies them, and `causal_past`
//! replays a frontier's past only.

use super::*;
use rekindle_governance::fixture::meta;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{AdmissionMode, Frontier};

fn cert(key: &SigningKey) -> GenesisCert {
    GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey(key.verifying_key().to_bytes()),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: None,
    }
}

#[test]
fn our_ops_chain_verify_and_replay_by_frontier() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = cert(&key);
    let state = GovernanceState::default();
    let empty = VerifiedSet::default();
    let (first, first_bytes) = build_op(
        "A",
        &cert,
        &state,
        &empty,
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("a"),
        Deps::Frontier,
        5,
    )
    .unwrap();
    assert_eq!(
        (first.op().seq, first.op().prev, first.lamport()),
        (0, None, 1)
    );
    let record = author_record(&cert, &state, first.author()).unwrap();
    let held_first = held(&first, record.clone(), first_bytes, OpFate::Applied);
    let set = verify_held("A", std::slice::from_ref(&held_first));
    assert_eq!(set.ops, vec![first.clone()]);
    let (second, second_bytes) = build_op(
        "A",
        &cert,
        &state,
        &set,
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("b"),
        Deps::Frontier,
        6,
    )
    .unwrap();
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
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("a"),
        Deps::Frontier,
        5,
    )
    .unwrap();
    let set = verify_held(
        "A",
        &[held(&first, "m".into(), first_bytes, OpFate::Applied)],
    );
    let (second, bytes) = build_op(
        "A",
        &cert,
        &state,
        &set,
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("b"),
        Deps::Frontier,
        6,
    )
    .unwrap();
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
    let (ours, ours_bytes) = build_op(
        "A",
        &cert,
        &state,
        &empty,
        crate::test_member::signer(&creator, &crate::test_member::device_of(&creator)),
        meta("a"),
        Deps::Frontier,
        5,
    )
    .unwrap();
    // A non-member's write verifies (signature, chain) but the merge refuses it.
    let (theirs, theirs_bytes) = build_op(
        "A",
        &cert,
        &state,
        &empty,
        crate::test_member::signer(&stranger, &crate::test_member::device_of(&stranger)),
        meta("b"),
        Deps::Frontier,
        5,
    )
    .unwrap();
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
        device_key: key.verifying_key().to_bytes(),
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
            owner_proof: vec![0; 64],
            first_device: None,
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
            uses: 0,
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

#[test]
fn an_equivocators_ops_from_the_fork_on_are_equivocation_and_its_earlier_ops_stay_applied() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = cert(&key);
    let state = GovernanceState::default();
    let (first, first_bytes) = build_op(
        "A",
        &cert,
        &state,
        &VerifiedSet::default(),
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("a"),
        Deps::Frontier,
        5,
    )
    .unwrap();
    let held_first = held(&first, "m".into(), first_bytes, OpFate::Applied);
    let base = verify_held("A", std::slice::from_ref(&held_first));
    // A restored backup: two different ops at seq 1, both after `first`.
    let (left, left_bytes) = build_op(
        "A",
        &cert,
        &state,
        &base,
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("b"),
        Deps::Frontier,
        6,
    )
    .unwrap();
    let (right, right_bytes) = build_op(
        "A",
        &cert,
        &state,
        &base,
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("c"),
        Deps::Frontier,
        7,
    )
    .unwrap();
    assert_eq!((left.op().seq, right.op().seq), (1, 1));
    let set = verify_held(
        "A",
        &[
            held_first,
            held(&left, "m".into(), left_bytes, OpFate::Applied),
            held(&right, "m".into(), right_bytes, OpFate::Applied),
        ],
    );
    let outcome = merge::merge_outcome(&cert, &set.ops);
    assert_eq!(outcome.equivocators, vec![(first.author().clone(), 1)]);
    let verdicts = fates(&set, &outcome);
    let of = |hash| verdicts.iter().find(|v| v.hash == hash).unwrap().fate;
    assert_eq!(of(first.hash()), OpFate::Applied);
    assert_eq!(of(left.hash()), OpFate::Equivocation);
    assert_eq!(of(right.hash()), OpFate::Equivocation);

    // Review S7m part 5: an equivocator's pre-fork op the merge refused or
    // holds for a dep keeps that fate; only its ops from the fork on are
    // `Equivocation`.
    let mut refused = outcome.clone();
    refused.applied.retain(|hash| *hash != first.hash());
    refused.refused.push(first.hash());
    let verdicts = fates(&set, &refused);
    let of = |hash| verdicts.iter().find(|v| v.hash == hash).unwrap().fate;
    assert_eq!(of(first.hash()), OpFate::Quarantined);
    assert_eq!(of(left.hash()), OpFate::Equivocation);
    let mut pending = outcome;
    pending.applied.retain(|hash| *hash != first.hash());
    pending.pending.push(first.hash());
    let verdicts = fates(&set, &pending);
    let of = |hash| verdicts.iter().find(|v| v.hash == hash).unwrap().fate;
    assert_eq!(of(first.hash()), OpFate::PendingDeps);
    assert_eq!(of(right.hash()), OpFate::Equivocation);
}

/// An op the merge masks after its author's removal (plan E3.2 S7c.3a)
/// is held quarantined, like a refused one, never applied.
#[test]
fn a_masked_op_is_held_quarantined() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = cert(&key);
    let state = GovernanceState::default();
    let empty = VerifiedSet::default();
    let (ours, bytes) = build_op(
        "A",
        &cert,
        &state,
        &empty,
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("a"),
        Deps::Frontier,
        5,
    )
    .unwrap();
    let set = verify_held("A", &[held(&ours, "m".into(), bytes, OpFate::Applied)]);
    let mut outcome = merge::merge_outcome(&cert, &set.ops);
    outcome.applied.clear();
    let cause = ours.hash();
    outcome.quarantined.push((
        ours.hash(),
        merge::QuarantineReason::AfterAuthorRemoval { cause },
    ));
    let verdicts = fates(&set, &outcome);
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].fate, OpFate::Quarantined);
}

/// Plan E3.2 S7c.5: an op whose fate moves to quarantined raises one
/// `GovernanceOpQuarantined`, with its reason; once held quarantined it
/// raises none on the next merge.
#[test]
fn a_quarantined_op_raises_one_governance_op_quarantined_event() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = cert(&key);
    let state = GovernanceState::default();
    let empty = VerifiedSet::default();
    let (ours, bytes) = build_op(
        "A",
        &cert,
        &state,
        &empty,
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        meta("a"),
        Deps::Frontier,
        5,
    )
    .unwrap();
    let applied = held(&ours, "m".into(), bytes.clone(), OpFate::Applied);
    let set = verify_held("A", std::slice::from_ref(&applied));
    let mut outcome = merge::merge_outcome(&cert, &set.ops);
    outcome.applied.clear();
    let reason = merge::QuarantineReason::AfterAuthorRemoval { cause: ours.hash() };
    outcome.quarantined.push((ours.hash(), reason.clone()));

    let events = crate::quarantine::newly_quarantined("A", &[applied], &outcome);
    assert_eq!(events.len(), 1, "one event for the move");
    let crate::event::GovernanceRuntimeEvent::GovernanceOpQuarantined {
        community_id,
        op,
        author,
        reason: got,
    } = &events[0]
    else {
        panic!("not GovernanceOpQuarantined");
    };
    assert_eq!(
        (community_id.as_str(), *op, author, got),
        ("A", ours.hash(), ours.author(), &reason)
    );

    let kept = held(&ours, "m".into(), bytes, OpFate::Quarantined);
    assert!(
        crate::quarantine::newly_quarantined("A", &[kept], &outcome).is_empty(),
        "already held quarantined: shown once"
    );
}

/// A restored backup that holds both of its ops at one seq publishes both
/// as its frontier's fork proof (D15, plan E3.2 S7d): its op log keeps one
/// op per seq, so a frontier naming only heads hid the branch the log lost.
/// Another author's frontier, or ours with no fork, carries no proof.
#[test]
fn our_own_fork_goes_out_as_the_frontiers_fork_proof() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = cert(&key);
    let state = GovernanceState::default();
    let build = |set: &VerifiedSet, name: &str, at: u64| {
        build_op(
            "A",
            &cert,
            &state,
            set,
            crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
            meta(name),
            Deps::Frontier,
            at,
        )
        .unwrap()
    };
    let (first, first_bytes) = build(&VerifiedSet::default(), "a", 5);
    let held_first = held(&first, "m".into(), first_bytes, OpFate::Applied);
    let base = verify_held("A", std::slice::from_ref(&held_first));
    let (left, left_bytes) = build(&base, "b", 6);
    let (right, right_bytes) = build(&base, "c", 7);
    let me = first.author().clone();
    let clean = own_frontier(&cert, &base, std::slice::from_ref(&held_first), Some(&me));
    assert!(clean.fork_proof.is_empty(), "no fork, no proof");
    let held_all = [
        held_first,
        held(&left, "m".into(), left_bytes.clone(), OpFate::Applied),
        held(&right, "m".into(), right_bytes.clone(), OpFate::Applied),
    ];
    let set = verify_held("A", &held_all);
    let ours = own_frontier(&cert, &set, &held_all, Some(&me));
    let mut proof = ours.fork_proof.clone();
    proof.sort();
    let mut want = vec![left_bytes, right_bytes];
    want.sort();
    assert_eq!(proof, want, "both branches of our fork");
    let encoded = rekindle_codec::governance_op::encode_frontier(&ours).unwrap();
    assert_eq!(
        rekindle_codec::governance_op::decode_frontier(&encoded).unwrap(),
        ours,
        "the proof survives the wire"
    );
    let other = PseudonymKey([9; 32]);
    assert!(own_frontier(&cert, &set, &held_all, Some(&other))
        .fork_proof
        .is_empty());
    assert!(own_frontier(&cert, &set, &held_all, None)
        .fork_proof
        .is_empty());
}

/// Two fork ops near the op bound never fit one frontier subkey beside the
/// heads (plan E3.2 S7d): the proof keeps what fits, the largest left out,
/// both stay named in `fork_refs`, and the frontier still encodes, so its
/// heads are written.
#[test]
fn a_fork_proof_too_large_for_the_subkey_keeps_what_fits() {
    use rekindle_types::dht_layout::member_record::MAX_SUBKEY_VALUE_LEN;
    let op_ref = |b: u8| OpRef {
        op_hash: OpHash([b; 32]),
        device_key: [3; 32],
        author: PseudonymKey([3; 32]),
        author_record: "VLD0:m".into(),
        author_seq: 6,
    };
    let small = vec![1u8; MAX_SUBKEY_VALUE_LEN / 2];
    let large = vec![2u8; MAX_SUBKEY_VALUE_LEN / 2 + 1];
    let frontier = Frontier {
        heads: vec![op_ref(1), op_ref(2)],
        roster_checksum: rekindle_types::governance::RosterChecksum([4; 32]),
        fork_proof: vec![large, small.clone()],
        fork_refs: vec![op_ref(1), op_ref(2)],
    };
    let fitted = fit_fork_proof(frontier.clone());
    let bytes = rekindle_codec::governance_op::encode_frontier(&fitted).expect("it fits");
    assert!(bytes.len() <= MAX_SUBKEY_VALUE_LEN);
    assert_eq!(fitted.fork_proof, vec![small], "the largest is left out");
    assert_eq!(fitted.fork_refs, frontier.fork_refs, "both stay named");
    assert_eq!(fitted.heads, frontier.heads);
    let fits = Frontier {
        fork_proof: vec![vec![5u8; 64], vec![6u8; 64]],
        ..frontier
    };
    assert_eq!(
        fit_fork_proof(fits.clone()),
        fits,
        "a proof that fits is kept whole"
    );
}
