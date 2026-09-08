//! Test modules for rekindle-transport.
//!
//! Tests that require a live Veilid node are gated behind integration
//! test infrastructure. Unit tests here cover payload serialization.
//! Frame, envelope and voice crypto tests are inline in their
//! respective modules. Gossip moved to `tests/gossip_wire_compat.rs`
//! when the postcard `GossipPayload` was deleted — it tests the Cap'n
//! Proto envelope both tracks actually exchange.

pub mod mock_node;

use crate::frame::TypeId;
use crate::payload::dm::{deserialize_dm, dm_type_id, serialize_dm, DmPayload};
use crate::payload::rpc::*;
use crate::payload::voice::VoicePayload;

#[test]
fn dm_roundtrip_direct_message() {
    let payload = DmPayload::DirectMessage {
        body: b"hello world".to_vec(),
        reply_to: None,
    };
    assert_eq!(dm_type_id(&payload), TypeId::DmMessage);
    let bytes = serialize_dm(&payload).unwrap();
    let back = deserialize_dm(TypeId::DmMessage, &bytes).unwrap();
    match back {
        DmPayload::DirectMessage { body, .. } => assert_eq!(body, b"hello world"),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn dm_roundtrip_friend_request() {
    let payload = DmPayload::FriendRequest {
        display_name: "Alice".into(),
        message: "Hi!".into(),
        prekey_bundle: vec![1, 2, 3],
        profile_dht_key: "VLD0:abc".into(),
        route_blob: vec![4, 5, 6],
        mailbox_dht_key: "VLD0:def".into(),
        invite_id: Some("inv_01".into()),
    };
    assert_eq!(dm_type_id(&payload), TypeId::FriendRequest);
    let bytes = serialize_dm(&payload).unwrap();
    let back = deserialize_dm(TypeId::FriendRequest, &bytes).unwrap();
    match back {
        DmPayload::FriendRequest {
            display_name,
            invite_id,
            ..
        } => {
            assert_eq!(display_name, "Alice");
            assert_eq!(invite_id.as_deref(), Some("inv_01"));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn voice_payload_roundtrip() {
    let payload = VoicePayload {
        sender_key_hex: "deadbeef".into(),
        sequence: 42,
        timestamp: 1_234_567_890,
        encrypted_audio: vec![0xAB; 100],
        hmac: [0x42; 16],
        signature: vec![0xCC; 64],
    };
    let bytes = postcard::to_stdvec(&payload).unwrap();
    let back: VoicePayload = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(back.sender_key_hex, "deadbeef");
    assert_eq!(back.sequence, 42);
    assert_eq!(back.encrypted_audio.len(), 100);
    assert_eq!(back.hmac, [0x42; 16]);
}

#[test]
fn rpc_bootstrap_roundtrip() {
    let req = BootstrapRequest {
        joiner_pseudonym: "joiner_abc".into(),
        governance_key: "VLD0:gov_key".into(),
    };
    let bytes = postcard::to_stdvec(&req).unwrap();
    let back: BootstrapRequest = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(back.joiner_pseudonym, "joiner_abc");
}

#[test]
fn call_response_roundtrip() {
    let resp = CallResponse::Ok(b"response data".to_vec());
    let bytes = serialize_call_response(&resp);
    let back: CallResponse = postcard::from_bytes(&bytes).unwrap();
    match back {
        CallResponse::Ok(data) => assert_eq!(data, b"response data"),
        _ => panic!("wrong variant"),
    }
}
