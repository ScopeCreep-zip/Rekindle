use super::*;
use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
use rekindle_types::governance::OpHash;
use rekindle_types::id::PseudonymKey;

/// A Veilid record key at its longest text form.
fn record_key(i: u8) -> String {
    format!(
        "VLD0:{}:{}",
        "A".repeat(43),
        char::from(b'a' + i % 26).to_string().repeat(43)
    )
}

fn op_ref(i: u8) -> OpRef {
    OpRef {
        op_hash: OpHash([i; 32]),
        device_key: [i.wrapping_add(1); 32],
        author: PseudonymKey([i.wrapping_add(1); 32]),
        author_record: record_key(i),
        author_seq: u64::MAX,
    }
}

#[test]
fn the_largest_response_fits_the_app_call_cap() {
    let max_frontier = u8::try_from(MAX_FRONTIER_REFS).unwrap();
    let payload = ControlPayload::BootstrapResponseV2 {
        anchor: record_key(200),
        frontier: (0..max_frontier).map(op_ref).collect(),
    };
    let bytes = encode_response(payload.clone()).expect("fits");
    assert!(
        bytes.len() < MAX_BOOTSTRAP_RESPONSE_LEN,
        "{} B",
        bytes.len()
    );
    let back = rekindle_codec::capnp_envelope::try_decode_community_envelope(&bytes)
        .unwrap()
        .unwrap();
    let Some(ControlPayload::BootstrapResponseV2 { frontier, .. }) = back.into_control() else {
        panic!("not a bootstrapResponseV2");
    };
    assert_eq!(frontier.len(), MAX_FRONTIER_REFS);
}

/// A re-join `MemberJoined` (seq 2) by `author`, as held.
fn rejoin(author: PseudonymKey) -> (HeldOp, OpRef) {
    let op = rekindle_types::governance::GovernanceOp {
        device_key: author.0,
        author: author.clone(),
        seq: 2,
        prev: None,
        deps: Vec::new(),
        issued_at: 1,
        body: rekindle_types::governance::GovernanceEntry::MemberJoined {
            member_record: record_key(7),
            member_record_owner: "o".into(),
            invite_proof: None,
            owner_proof: vec![0; 64],
            first_device: None,
        },
        sig: [0; 64],
    };
    let held = HeldOp {
        hash: OpHash([9; 32]),
        device_key: author.0,
        author: author.clone(),
        author_record: record_key(7),
        seq: 2,
        lamport: 5,
        issued_at: 1,
        encoded: rekindle_codec::governance_op::encode_op(&op).expect("encodes"),
        fate: op_log::OpFate::Applied,
    };
    let op_ref = OpRef {
        op_hash: held.hash,
        device_key: author.0,
        author,
        author_record: record_key(7),
        author_seq: 2,
    };
    (held, op_ref)
}

fn member(hash: OpHash) -> rekindle_governance::state::MemberState {
    rekindle_governance::state::MemberState {
        member_record: record_key(7),
        member_record_owner: "o".into(),
        joined: rekindle_governance::op::OpStamp { lamport: 5, hash },
    }
}

/// Walk M87: the inviter merged the peer's re-join from gossip before
/// its `BootstrapRequest` arrived, and refused it as a member's second
/// join; a re-join admitted by this very op is the joiner's, while one
/// by a member admitted by another op is not.
#[test]
fn a_rejoin_already_merged_is_the_joiners_member_joined() {
    let author = PseudonymKey([4; 32]);
    let (held, op_ref) = rejoin(author.clone());
    let mut state = rekindle_governance::state::GovernanceState::default();
    assert!(is_joiners_member_joined(&held, &op_ref, &state));
    state.members.insert(author.clone(), member(held.hash));
    assert!(is_joiners_member_joined(&held, &op_ref, &state));
    state.members.insert(author, member(OpHash([1; 32])));
    assert!(!is_joiners_member_joined(&held, &op_ref, &state));
}

#[test]
fn a_frontier_over_the_bound_is_refused() {
    let over = u8::try_from(MAX_FRONTIER_REFS + 1).unwrap();
    let payload = ControlPayload::BootstrapResponseV2 {
        anchor: record_key(0),
        frontier: (0..over).map(op_ref).collect(),
    };
    assert!(encode_response(payload).is_err());
}
