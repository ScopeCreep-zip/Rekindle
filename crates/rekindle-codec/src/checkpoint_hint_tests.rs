use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;
use rekindle_types::governance::{OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::*;
use crate::capnp_envelope::{encode_community_envelope, try_decode_community_envelope};
use crate::community::envelope::{CommunityEnvelope, ControlPayload};

fn request(key: &SigningKey, invite_code: Option<&str>) -> CheckpointHintRequest {
    let request = CheckpointHintRequest {
        community: format!("VLD0:{}", "A".repeat(150)),
        height: u64::MAX,
        digest: [7; 32],
        requester: key.verifying_key().to_bytes(),
        invite: None,
    };
    match invite_code {
        Some(code) => request.with_invite_code(code),
        None => request,
    }
}

/// The longest locator: a full member record key and the largest seq.
fn op_ref(i: u8) -> OpRef {
    OpRef {
        op_hash: OpHash([i; 32]),
        device_key: [i.wrapping_add(1); 32],
        author: PseudonymKey([i.wrapping_add(1); 32]),
        author_record: format!("VLD0:{}:{}", "A".repeat(43), "b".repeat(43)),
        author_seq: u64::MAX,
    }
}

/// A member's and a joiner's request round-trip through the envelope, and
/// the signature binds the invite code hash and the height.
#[test]
fn a_hint_request_round_trips_and_its_signature_binds_it() {
    let key = SigningKey::from_bytes(&[3; 32]);
    for invite in [None, Some("ab".repeat(32))] {
        let want = request(&key, invite.as_deref());
        let encoded = want.encode(&key).unwrap();
        let envelope = encode_community_envelope(&CommunityEnvelope::control(
            ControlPayload::CheckpointHintRequest { request: encoded },
        ))
        .unwrap();
        let Some(ControlPayload::CheckpointHintRequest { request: got }) =
            try_decode_community_envelope(&envelope)
                .ok()
                .flatten()
                .and_then(CommunityEnvelope::into_control)
        else {
            panic!("not a checkpoint hint request");
        };
        assert_eq!(decode_checkpoint_hint_request(&got).unwrap(), want);
    }
    let other = SigningKey::from_bytes(&[4; 32]);
    assert!(request(&key, None).encode(&other).is_err());
    let sig = sign(&key, &request(&key, None).signing_bytes());
    let mut builder = capnp::message::Builder::new_default();
    {
        let joiner = request(&key, Some("cd"));
        let mut b = builder.init_root::<checkpoint_hint_request::Builder<'_>>();
        b.set_community(&joiner.community);
        b.set_height(joiner.height);
        b.set_digest(&joiner.digest);
        b.set_requester(&joiner.requester);
        let invite = joiner.invite.as_ref().unwrap();
        b.set_invite_code_hash(&invite.code_hash);
        b.set_invite_tag(&invite.tag);
        b.set_sig(&sig);
    }
    assert!(decode_checkpoint_hint_request(&pack(&builder)).is_err());
}

/// The largest answer, a checkpoint and [`MAX_HINT_ENDORSEMENTS`] longest
/// locators, fits the 32,768 B `app_call` cap in its envelope; one more
/// endorsement is refused.
#[test]
fn the_largest_hint_response_fits_the_app_call_cap() {
    let response = CheckpointHintResponse {
        checkpoint: Some(op_ref(255)),
        endorsements: (0..MAX_HINT_ENDORSEMENTS)
            .map(|i| op_ref(u8::try_from(i).unwrap()))
            .collect(),
    };
    let encoded = response.encode().unwrap();
    let envelope = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::CheckpointHintResponse { response: encoded },
    ))
    .unwrap();
    assert!(
        envelope.len() <= MAX_APP_MESSAGE_LEN,
        "{} B",
        envelope.len()
    );
    let Some(ControlPayload::CheckpointHintResponse { response: got }) =
        try_decode_community_envelope(&envelope)
            .ok()
            .flatten()
            .and_then(CommunityEnvelope::into_control)
    else {
        panic!("not a checkpoint hint response");
    };
    assert_eq!(decode_checkpoint_hint_response(&got).unwrap(), response);
    let mut over = response;
    over.endorsements.push(op_ref(0));
    assert!(over.encode().is_err());
    let none = CheckpointHintResponse::default();
    assert_eq!(
        decode_checkpoint_hint_response(&none.encode().unwrap()).unwrap(),
        none
    );
}
