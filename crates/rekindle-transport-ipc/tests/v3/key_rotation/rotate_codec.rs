use rekindle_transport_ipc::v3::codec::channel::rotate::{
    RotateInitPayload, RotateCommitPayload, encode_init, decode_init, encode_commit, decode_commit,
};

#[test]
fn rotate_init_roundtrip() {
    let original = RotateInitPayload {
        rotation_id: uuid::Uuid::nil(),
        initiator_generation: 5,
        phase_2_deadline_ms: 30_000,
        new_static_pub: [0xAA; 32],
        rotation_chain_secret: [0xBB; 32],
        transcript_anchor: [0xCC; 32],
    };
    let encoded = encode_init(&original);
    let decoded = decode_init(&encoded).expect("decode failed");
    assert_eq!(decoded.rotation_id, original.rotation_id);
    assert_eq!(decoded.initiator_generation, original.initiator_generation);
    assert_eq!(decoded.phase_2_deadline_ms, original.phase_2_deadline_ms);
    assert_eq!(decoded.new_static_pub, original.new_static_pub);
    assert_eq!(decoded.rotation_chain_secret, original.rotation_chain_secret);
    assert_eq!(decoded.transcript_anchor, original.transcript_anchor);
}

#[test]
fn rotate_commit_roundtrip() {
    let original = RotateCommitPayload {
        rotation_id: uuid::Uuid::nil(),
        responder_generation: 7,
        new_static_pub: [0xDD; 32],
        rotation_chain_secret: [0xEE; 32],
        transcript_anchor: [0xFF; 32],
    };
    let encoded = encode_commit(&original);
    let decoded = decode_commit(&encoded).expect("decode failed");
    assert_eq!(decoded.rotation_id, original.rotation_id);
    assert_eq!(decoded.responder_generation, original.responder_generation);
    assert_eq!(decoded.new_static_pub, original.new_static_pub);
    assert_eq!(decoded.rotation_chain_secret, original.rotation_chain_secret);
    assert_eq!(decoded.transcript_anchor, original.transcript_anchor);
}

#[test]
fn rotate_init_with_different_generations() {
    let p1 = RotateInitPayload {
        rotation_id: uuid::Uuid::nil(),
        initiator_generation: 0,
        phase_2_deadline_ms: 30_000,
        new_static_pub: [0xAA; 32],
        rotation_chain_secret: [0xBB; 32],
        transcript_anchor: [0xCC; 32],
    };
    let mut p2 = p1.clone();
    p2.initiator_generation = 999;

    let e1 = encode_init(&p1);
    let e2 = encode_init(&p2);
    assert_ne!(e1, e2, "different generations must produce different encodings");
}
