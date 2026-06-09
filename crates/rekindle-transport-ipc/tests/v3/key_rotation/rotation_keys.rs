use rekindle_transport_ipc::v3::crypto::keys::{derive_all_keys, derive_rotation_keys};

#[test]
fn rotation_keys_differ_from_handshake_keys() {
    let handshake_hash = [0xAA; 32];
    let combined_secret = [0xBB; 32];
    let hk = derive_all_keys(&handshake_hash);
    let rk = derive_rotation_keys(&combined_secret);
    assert_ne!(hk.envelope_d2l, rk.envelope_d2l);
    assert_ne!(hk.stream_d2l, rk.stream_d2l);
    assert_ne!(hk.handoff, rk.handoff);
}

#[test]
fn rotation_keys_are_deterministic() {
    let combined = [0xCC; 32];
    let a = derive_rotation_keys(&combined);
    let b = derive_rotation_keys(&combined);
    assert_eq!(a.envelope_d2l, b.envelope_d2l);
    assert_eq!(a.envelope_l2d, b.envelope_l2d);
    assert_eq!(a.header_d2l, b.header_d2l);
    assert_eq!(a.header_l2d, b.header_l2d);
    assert_eq!(a.stream_d2l, b.stream_d2l);
    assert_eq!(a.stream_l2d, b.stream_l2d);
    assert_eq!(a.audit_d2l, b.audit_d2l);
    assert_eq!(a.audit_l2d, b.audit_l2d);
    assert_eq!(a.handoff, b.handoff);
}

#[test]
fn rotation_keys_all_distinct() {
    let rk = derive_rotation_keys(&[0xDD; 32]);
    let all: Vec<&[u8; 32]> = vec![
        &rk.envelope_d2l, &rk.envelope_l2d,
        &rk.header_d2l, &rk.header_l2d,
        &rk.stream_d2l, &rk.stream_l2d,
        &rk.audit_d2l, &rk.audit_l2d,
        &rk.handoff,
    ];
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            if i != j {
                assert_ne!(*a, *b, "rotation keys {i} and {j} collide");
            }
        }
    }
}

#[test]
fn different_combined_secret_produces_different_keys() {
    let a = derive_rotation_keys(&[0x11; 32]);
    let b = derive_rotation_keys(&[0x22; 32]);
    assert_ne!(a.envelope_d2l, b.envelope_d2l);
    assert_ne!(a.stream_d2l, b.stream_d2l);
    assert_ne!(a.handoff, b.handoff);
}

#[test]
fn combined_secret_order_matters() {
    let initiator_secret = [0xAA; 32];
    let responder_secret = [0xBB; 32];

    let mut forward = [0u8; 64];
    forward[..32].copy_from_slice(&initiator_secret);
    forward[32..].copy_from_slice(&responder_secret);

    let mut reversed = [0u8; 64];
    reversed[..32].copy_from_slice(&responder_secret);
    reversed[32..].copy_from_slice(&initiator_secret);

    let fwd_hash = blake3::hash(&forward);
    let rev_hash = blake3::hash(&reversed);

    let fk = derive_rotation_keys(fwd_hash.as_bytes());
    let rk = derive_rotation_keys(rev_hash.as_bytes());
    assert_ne!(fk.envelope_d2l, rk.envelope_d2l,
        "initiator||responder must produce different keys than responder||initiator");
}
