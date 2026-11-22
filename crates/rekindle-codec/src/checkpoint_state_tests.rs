use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

use super::*;
use crate::capnp_envelope::{encode_community_envelope, try_decode_community_envelope};
use crate::community::envelope::{CommunityEnvelope, ControlPayload};

fn request(key: &SigningKey) -> CheckpointStateRequest {
    CheckpointStateRequest {
        community: format!("VLD0:{}", "A".repeat(150)),
        height: u64::MAX,
        digest: [7; 32],
        offset: u32::MAX,
        requester: key.verifying_key().to_bytes(),
        invite: None,
    }
}

/// Bytes packing cannot shrink: BLAKE3's XOF output.
fn incompressible(len: usize) -> Vec<u8> {
    let mut out = vec![0; len];
    blake3::Hasher::new().finalize_xof().fill(&mut out);
    out
}

#[test]
fn a_request_round_trips_through_the_envelope() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let encoded = request(&key).encode(&key).unwrap();
    let envelope = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::CheckpointStateRequest { request: encoded },
    ))
    .unwrap();
    let Some(ControlPayload::CheckpointStateRequest { request: got }) =
        try_decode_community_envelope(&envelope)
            .ok()
            .flatten()
            .and_then(CommunityEnvelope::into_control)
    else {
        panic!("not a checkpoint state request");
    };
    assert_eq!(
        decode_checkpoint_state_request(&got).unwrap(),
        request(&key)
    );
}

/// The signature binds every field: a request re-pointed at another offset
/// is refused, as is one signed by a key that is not the requester's.
#[test]
fn a_request_whose_signature_does_not_bind_it_is_refused() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let other = SigningKey::from_bytes(&[4; 32]);
    assert!(request(&key).encode(&other).is_err());
    let mut changed = request(&key);
    changed.offset = 0;
    let sig = sign(&key, &request(&key).signing_bytes());
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut b = builder.init_root::<checkpoint_state_request::Builder<'_>>();
        b.set_community(&changed.community);
        b.set_height(changed.height);
        b.set_digest(&changed.digest);
        b.set_offset(changed.offset);
        b.set_requester(&changed.requester);
        b.set_sig(&sig);
    }
    assert!(decode_checkpoint_state_request(&pack(&builder)).is_err());
}

/// A full chunk of incompressible bytes, in its envelope, fits the 32,768 B
/// `app_call` cap (plan E3.2 S7c.3: "under the 32,768 B app_call cap").
#[test]
fn a_full_chunk_fits_the_app_call_cap() {
    let response = CheckpointStateResponse {
        total: u32::MAX,
        bytes: incompressible(SNAPSHOT_CHUNK_MAX),
    };
    let encoded = response.encode().unwrap();
    let envelope = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::CheckpointStateResponse { response: encoded },
    ))
    .unwrap();
    assert!(
        envelope.len() <= MAX_APP_MESSAGE_LEN,
        "{} B",
        envelope.len()
    );
    let Some(ControlPayload::CheckpointStateResponse { response: got }) =
        try_decode_community_envelope(&envelope)
            .ok()
            .flatten()
            .and_then(CommunityEnvelope::into_control)
    else {
        panic!("not a checkpoint state response");
    };
    assert_eq!(decode_checkpoint_state_response(&got).unwrap(), response);
}

/// A joiner's request carries the invite it holds (plan E3.2 S7c.5): it
/// round-trips, and the signature binds its code hash, so a member's request
/// cannot be re-pointed at an invite nor a joiner's at another.
#[test]
fn a_joiners_invite_code_hash_round_trips_and_is_signed() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let joiner = request(&key).with_invite_code(&"ab".repeat(16));
    let encoded = joiner.encode(&key).unwrap();
    assert!(encoded.len() <= CHECKPOINT_STATE_REQUEST_MAX_BYTES);
    assert_eq!(decode_checkpoint_state_request(&encoded).unwrap(), joiner);
    let sig = sign(&key, &request(&key).signing_bytes());
    let invite = joiner.invite.as_ref().unwrap();
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut b = builder.init_root::<checkpoint_state_request::Builder<'_>>();
        b.set_community(&joiner.community);
        b.set_height(joiner.height);
        b.set_digest(&joiner.digest);
        b.set_offset(joiner.offset);
        b.set_requester(&joiner.requester);
        b.set_sig(&sig);
        b.set_invite_code_hash(&invite.code_hash);
        b.set_invite_tag(&invite.tag);
    }
    assert!(decode_checkpoint_state_request(&pack(&builder)).is_err());
}

/// Plan E3.2 S7c.8 (2): the public code hash alone proves nothing. A
/// request tagged under the invite code is proven by that code after its
/// round trip; one naming the same code hash with a tag not made from the
/// code, or under another code, is not, and a tag binds the bytes asked.
#[test]
fn only_the_invite_code_proves_a_joiners_request() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let code = "ab".repeat(16);
    let joiner = request(&key).with_invite_code(&code);
    let decoded = decode_checkpoint_state_request(&joiner.encode(&key).unwrap()).unwrap();
    assert!(decoded.invite_proven_by(&code));
    assert!(!decoded.invite_proven_by(&"cd".repeat(16)));
    let hash_only = CheckpointStateRequest {
        invite: Some(PreJoinInvite {
            code_hash: rekindle_secrets::invite::hash_invite_code(&code),
            tag: [0; 32],
        }),
        ..request(&key)
    };
    let decoded = decode_checkpoint_state_request(&hash_only.encode(&key).unwrap()).unwrap();
    assert!(
        !decoded.invite_proven_by(&code),
        "the hash without the code"
    );
    let moved = CheckpointStateRequest {
        offset: 0,
        ..joiner.clone()
    };
    assert!(!moved.invite_proven_by(&code), "the tag binds the offset");
    assert!(!request(&key).invite_proven_by(&code), "a member's request");
}

#[test]
fn a_chunk_over_the_bound_is_refused() {
    let response = CheckpointStateResponse {
        total: 1,
        bytes: vec![1; SNAPSHOT_CHUNK_MAX + 1],
    };
    assert!(response.encode().is_err());
}
