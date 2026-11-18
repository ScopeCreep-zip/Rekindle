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
        invite_code_hash: None,
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
    let envelope = encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::CheckpointStateRequest { request: encoded },
    ))
    .unwrap();
    let Ok(Some(CommunityEnvelope::Control(ControlPayload::CheckpointStateRequest {
        request: got,
    }))) = try_decode_community_envelope(&envelope)
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
    let envelope = encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::CheckpointStateResponse { response: encoded },
    ))
    .unwrap();
    assert!(
        envelope.len() <= MAX_APP_MESSAGE_LEN,
        "{} B",
        envelope.len()
    );
    let Ok(Some(CommunityEnvelope::Control(ControlPayload::CheckpointStateResponse {
        response: got,
    }))) = try_decode_community_envelope(&envelope)
    else {
        panic!("not a checkpoint state response");
    };
    assert_eq!(decode_checkpoint_state_response(&got).unwrap(), response);
}

/// A joiner's request carries the code hash of the invite it holds (plan
/// E3.2 S7c.5): it round-trips, and the signature binds it, so a member's
/// request cannot be re-pointed at an invite nor a joiner's at another.
#[test]
fn a_joiners_invite_code_hash_round_trips_and_is_signed() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let joiner = CheckpointStateRequest {
        invite_code_hash: Some("ab".repeat(32)),
        ..request(&key)
    };
    let encoded = joiner.encode(&key).unwrap();
    assert!(encoded.len() <= CHECKPOINT_STATE_REQUEST_MAX_BYTES);
    assert_eq!(decode_checkpoint_state_request(&encoded).unwrap(), joiner);
    let sig = sign(&key, &request(&key).signing_bytes());
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut b = builder.init_root::<checkpoint_state_request::Builder<'_>>();
        b.set_community(&joiner.community);
        b.set_height(joiner.height);
        b.set_digest(&joiner.digest);
        b.set_offset(joiner.offset);
        b.set_requester(&joiner.requester);
        b.set_sig(&sig);
        b.set_invite_code_hash(joiner.invite_code_hash.as_deref().unwrap());
    }
    assert!(decode_checkpoint_state_request(&pack(&builder)).is_err());
}

#[test]
fn a_chunk_over_the_bound_is_refused() {
    let response = CheckpointStateResponse {
        total: 1,
        bytes: vec![1; SNAPSHOT_CHUNK_MAX + 1],
    };
    assert!(response.encode().is_err());
}
