//! Wire-identity proofs for the newtype-converted `ControlPayload`
//! variants. Every literal here (JSON values AND Cap'n Proto hex) was
//! captured by running the same assertions against the ORIGINAL
//! inline struct-variant enum — passing unchanged after the
//! conversion proves both wires are byte-identical.

use rekindle_types::video::Codec;
use serde_json::json;

use super::{CommunityEnvelope, ControlPayload};
use super::{
    MekTransferAckPayload, MekTransferPayload, VideoFragmentPayload, VideoParityFragmentPayload,
};
use crate::capnp_envelope::encode_community_envelope;

fn sample_video_fragment() -> ControlPayload {
    ControlPayload::VideoFragment(VideoFragmentPayload {
        channel_id: "ch1".into(),
        stream_id: [7u8; 16],
        frame_seq: 42,
        frag_index: 1,
        frag_total: 3,
        keyframe: true,
        codec: Codec::Vp9,
        timestamp: 123_456,
        mek_generation: 9,
        payload: vec![1, 2, 3],
        signature: vec![4, 5, 6],
    })
}

fn sample_parity() -> ControlPayload {
    ControlPayload::VideoParityFragment(VideoParityFragmentPayload {
        channel_id: "ch1".into(),
        stream_id: [7u8; 16],
        frame_seq: 42,
        parity_index: 0,
        parity_total: 2,
        data_count: 3,
        codec: Codec::Vp9,
        frame_len: 999,
        timestamp: 123_456,
        mek_generation: 9,
        payload: vec![9, 9],
        signature: vec![8, 8],
    })
}

fn sample_mek_transfer() -> ControlPayload {
    ControlPayload::MekTransfer(MekTransferPayload {
        community_id: "c1".into(),
        channel_id: Some("ch1".into()),
        generation: 5,
        sender_pseudonym: "p1".into(),
        wrapped_mek: vec![0xAA, 0xBB],
    })
}

fn sample_mek_ack() -> ControlPayload {
    ControlPayload::MekTransferAck(MekTransferAckPayload {
        community_id: "c1".into(),
        channel_id: None,
        generation: 5,
        requester_pseudonym: "p2".into(),
    })
}

#[test]
fn serde_json_wire_identity() {
    // This enum family has NO serde(rename_all): PascalCase variant
    // tags + snake_case fields. The payload structs use serde
    // defaults for exactly that reason.
    assert_eq!(
        serde_json::to_value(sample_mek_transfer()).unwrap(),
        json!({"type": "MekTransfer", "data": {
            "community_id": "c1", "channel_id": "ch1", "generation": 5,
            "sender_pseudonym": "p1", "wrapped_mek": [170, 187]
        }})
    );
    assert_eq!(
        serde_json::to_value(sample_mek_ack()).unwrap(),
        json!({"type": "MekTransferAck", "data": {
            "community_id": "c1", "generation": 5, "requester_pseudonym": "p2"
        }})
    );
    let v = serde_json::to_value(sample_video_fragment()).unwrap();
    assert_eq!(v["type"], "VideoFragment");
    assert_eq!(v["data"]["stream_id"][0], 7);
    assert_eq!(v["data"]["mek_generation"], 9);
    let p = serde_json::to_value(sample_parity()).unwrap();
    assert_eq!(p["type"], "VideoParityFragment");
    assert_eq!(p["data"]["data_count"], 3);
}

#[test]
fn capnp_wire_identity() {
    let expect = [
        ("video_fragment", sample_video_fragment(), "10115001010101500101013b500304712a0103010740e2010109110d22110d8211111a11111a07636831ff07070707070707070107070707070707070701020307040506"),
        ("parity", sample_parity(), "10125001010101500101013c500404612a020373e70340e20100000109110d22110d8211111211111207636831ff0707070707070707010707070707070707030909030808"),
        ("mek_transfer", sample_mek_transfer(), "100f5001010101500101011050020401010105110d1a110d22110d1a110d120363310763683103703103aabb"),
        ("mek_ack", sample_mek_ack(), "100c500101010150010101435002030000010511091a000011051a036331037032"),
    ];
    for (name, payload, hex_expected) in expect {
        let env = CommunityEnvelope::Control(payload);
        let bytes = encode_community_envelope(&env).expect(name);
        assert_eq!(
            hex::encode(&bytes),
            hex_expected,
            "capnp bytes changed for {name}"
        );
    }
}
