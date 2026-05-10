//! Test modules for rekindle-transport.
//!
//! Tests that require a live Veilid node are gated behind integration
//! test infrastructure. Unit tests here cover payload serialization.
//! Frame, envelope, voice crypto, and gossip tests are inline in their
//! respective modules.

pub mod mock_node;

use crate::payload::dm::{DmPayload, serialize_dm, deserialize_dm, dm_type_id};
use crate::payload::gossip::{GossipPayload, ControlPayload, SignedGossipEnvelope};
use crate::payload::voice::VoicePayload;
use crate::payload::rpc::*;
use crate::frame::TypeId;

#[test]
fn dm_roundtrip_typing() {
    let payload = DmPayload::Typing { typing: true };
    assert_eq!(dm_type_id(&payload), TypeId::DmTyping);
    let bytes = serialize_dm(&payload).unwrap();
    let back = deserialize_dm(TypeId::DmTyping, &bytes).unwrap();
    match back {
        DmPayload::Typing { typing } => assert!(typing),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn dm_roundtrip_friend_request_ack() {
    let payload = DmPayload::FriendRequestAck;
    assert_eq!(dm_type_id(&payload), TypeId::FriendRequestAck);
    let bytes = serialize_dm(&payload).unwrap();
    let back = deserialize_dm(TypeId::FriendRequestAck, &bytes).unwrap();
    assert!(matches!(back, DmPayload::FriendRequestAck));
}

#[test]
fn gossip_payload_roundtrip() {
    let payload = GossipPayload::MessageNotification {
        channel_id: "ch_01".into(),
        message_id: "msg_abc".into(),
        author_pseudonym: "pseudo_123".into(),
        subkey_index: 7,
        lamport_ts: 42,
        sequence: 3,
        content_hash: "abc123".into(),
        timestamp: 1234567890,
    };
    let bytes = postcard::to_stdvec(&payload).unwrap();
    let back: GossipPayload = postcard::from_bytes(&bytes).unwrap();
    match back {
        GossipPayload::MessageNotification { channel_id, message_id, .. } => {
            assert_eq!(channel_id, "ch_01");
            assert_eq!(message_id, "msg_abc");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn control_payload_roundtrip() {
    let payload = ControlPayload::MemberLeave { pseudonym_key: "abc123".into() };
    let bytes = postcard::to_stdvec(&payload).unwrap();
    let back: ControlPayload = postcard::from_bytes(&bytes).unwrap();
    match back {
        ControlPayload::MemberLeave { pseudonym_key } => assert_eq!(pseudonym_key, "abc123"),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn voice_payload_roundtrip() {
    let payload = VoicePayload {
        sender_key_hex: "deadbeef".into(),
        sequence: 42,
        timestamp: 1234567890,
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

#[test]
fn signed_gossip_envelope_dedup_key_message() {
    let payload = GossipPayload::MessageNotification {
        channel_id: "ch_01".into(),
        message_id: "unique_msg_id".into(),
        author_pseudonym: "p".into(),
        subkey_index: 0,
        lamport_ts: 1,
        sequence: 1,
        content_hash: "hash".into(),
        timestamp: 0,
    };
    let payload_bytes = postcard::to_stdvec(&payload).unwrap();
    let envelope = SignedGossipEnvelope {
        community_id: "c1".into(),
        sender_pseudonym: "sender".into(),
        payload_bytes,
        signature: vec![0; 64],
        ttl: 5,
        lamport_ts: 1,
    };
    assert_eq!(crate::payload::gossip::dedup_key(&envelope), "unique_msg_id");
}

#[test]
fn signed_gossip_envelope_private_detection() {
    let payload = GossipPayload::Control(ControlPayload::JoinAccepted {
        mek_encrypted: vec![],
        mek_generation: 0,
        member_registry_key: None,
        slot_index: None,
        wrapped_slot_seed: None,
    });
    let payload_bytes = postcard::to_stdvec(&payload).unwrap();
    let envelope = SignedGossipEnvelope {
        community_id: "c1".into(),
        sender_pseudonym: "s".into(),
        payload_bytes,
        signature: vec![0; 64],
        ttl: 5,
        lamport_ts: 1,
    };
    assert!(crate::payload::gossip::is_private(&envelope));
}

#[test]
fn message_notification_stays_compact() {
    let payload = GossipPayload::MessageNotification {
        channel_id: "ch01".into(),
        message_id: "m01".into(),
        author_pseudonym: "p01".into(),
        subkey_index: 7,
        lamport_ts: 42,
        sequence: 3,
        content_hash: "abc123".into(),
        timestamp: 1234567890,
    };
    let bytes = postcard::to_stdvec(&payload).unwrap();
    assert!(
        bytes.len() < 200,
        "MessageNotification should be compact (< 200 bytes), was {} bytes",
        bytes.len()
    );
}
