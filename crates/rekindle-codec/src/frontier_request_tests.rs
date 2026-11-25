use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;
use rekindle_types::governance::{Frontier, OpHash, OpRef, RosterChecksum};
use rekindle_types::id::PseudonymKey;

use super::*;
use crate::capnp_envelope::{encode_community_envelope, try_decode_community_envelope};
use crate::community::envelope::{CommunityEnvelope, ControlPayload};

fn request(key: &SigningKey) -> FrontierRequest {
    FrontierRequest {
        community: format!("VLD0:{}", "A".repeat(150)),
        requester: key.verifying_key().to_bytes(),
    }
}

/// The longest head: a full member record key and the largest seq.
fn head(i: u8) -> OpRef {
    OpRef {
        op_hash: OpHash([i; 32]),
        device_key: [i.wrapping_add(1); 32],
        author: PseudonymKey([i.wrapping_add(2); 32]),
        author_record: format!("VLD0:{}:{}", "A".repeat(43), "b".repeat(43)),
        author_seq: u64::MAX,
    }
}

fn frontier(heads: usize) -> Frontier {
    Frontier {
        heads: (0..heads).map(|i| head(u8::try_from(i).unwrap())).collect(),
        roster_checksum: RosterChecksum([9; 32]),
        fork_proof: Vec::new(),
        fork_refs: vec![head(200), head(201)],
    }
}

/// A request round-trips through the envelope, and its signature binds the
/// community and the requester.
#[test]
fn a_frontier_request_round_trips_and_its_signature_binds_it() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let encoded = request(&key).encode(&key).unwrap();
    assert!(encoded.len() <= FRONTIER_REQUEST_MAX_BYTES);
    let envelope = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::FrontierRequest { request: encoded },
    ))
    .unwrap();
    let Some(ControlPayload::FrontierRequest { request: got }) =
        try_decode_community_envelope(&envelope)
            .unwrap()
            .and_then(CommunityEnvelope::into_control)
    else {
        panic!("not a frontier request");
    };
    assert_eq!(decode_frontier_request(&got).unwrap(), request(&key));

    let other = SigningKey::from_bytes(&[4; 32]);
    assert!(
        request(&key).encode(&other).is_err(),
        "only the requester signs"
    );
    let mut tampered = request(&key).encode(&key).unwrap();
    let last = tampered.last_mut().unwrap();
    *last = if *last == 1 { 2 } else { *last ^ 1 };
    assert!(
        decode_frontier_request(&tampered).is_err(),
        "a broken signature is refused"
    );
}

/// A 64-head answer, the longest heads, fits the `app_call` cap with the
/// envelope around it and round-trips; 65 heads and fork proof ops are refused.
#[test]
fn a_64_head_frontier_response_fits_the_app_call_cap() {
    let full = frontier(MAX_FRONTIER_REFS);
    let encoded = encode_frontier_response(&full).unwrap();
    assert!(encoded.len() <= FRONTIER_RESPONSE_MAX_BYTES);
    let envelope = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::FrontierResponse { frontier: encoded },
    ))
    .unwrap();
    assert!(
        envelope.len() <= MAX_APP_MESSAGE_LEN,
        "{} B over the app_call cap",
        envelope.len()
    );
    let Some(ControlPayload::FrontierResponse { frontier: got }) =
        try_decode_community_envelope(&envelope)
            .unwrap()
            .and_then(CommunityEnvelope::into_control)
    else {
        panic!("not a frontier response");
    };
    assert_eq!(decode_frontier_response(&got).unwrap(), full);

    assert!(encode_frontier_response(&frontier(MAX_FRONTIER_REFS + 1)).is_err());
    let mut proof = frontier(1);
    proof.fork_proof = vec![vec![1; 16]];
    assert!(encode_frontier_response(&proof).is_err());
    let with_proof = crate::governance_op::encode_frontier(&proof).unwrap();
    assert!(decode_frontier_response(&with_proof).is_err());
}

/// An answer over the response cap is refused on both sides.
#[test]
fn a_frontier_response_over_its_cap_is_refused() {
    let oversized = vec![0_u8; FRONTIER_RESPONSE_MAX_BYTES + 1];
    assert!(decode_frontier_response(&oversized).is_err());
}
