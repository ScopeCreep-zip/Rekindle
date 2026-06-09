use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;

fn test_handshake_hash() -> [u8; 32] {
    [0xAA; 32]
}

fn other_handshake_hash() -> [u8; 32] {
    [0xBB; 32]
}

#[test]
fn derive_all_keys_returns_nine_keys() {
    let keys = derive_all_keys(&test_handshake_hash());
    // DerivedKeys has 9 fields; this test verifies the struct exists
    // and is constructable. If a field is missing, this fails to compile.
    let _: &[u8; 32] = &keys.envelope_d2l;
    let _: &[u8; 32] = &keys.envelope_l2d;
    let _: &[u8; 32] = &keys.header_d2l;
    let _: &[u8; 32] = &keys.header_l2d;
    let _: &[u8; 32] = &keys.stream_d2l;
    let _: &[u8; 32] = &keys.stream_l2d;
    let _: &[u8; 32] = &keys.audit_d2l;
    let _: &[u8; 32] = &keys.audit_l2d;
    let _: &[u8; 32] = &keys.handoff;
}

#[test]
fn all_derived_keys_are_distinct() {
    let keys = derive_all_keys(&test_handshake_hash());
    let all: Vec<&[u8; 32]> = vec![
        &keys.envelope_d2l, &keys.envelope_l2d,
        &keys.header_d2l, &keys.header_l2d,
        &keys.stream_d2l, &keys.stream_l2d,
        &keys.audit_d2l, &keys.audit_l2d,
        &keys.handoff,
    ];
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            if i != j {
                assert_ne!(
                    *a, *b,
                    "Keys at index {i} and {j} are identical. \
                     HKDF labels are colliding."
                );
            }
        }
    }
}

#[test]
fn derivation_is_deterministic() {
    let keys_a = derive_all_keys(&test_handshake_hash());
    let keys_b = derive_all_keys(&test_handshake_hash());
    assert_eq!(keys_a.envelope_d2l, keys_b.envelope_d2l);
    assert_eq!(keys_a.envelope_l2d, keys_b.envelope_l2d);
    assert_eq!(keys_a.header_d2l, keys_b.header_d2l);
    assert_eq!(keys_a.header_l2d, keys_b.header_l2d);
    assert_eq!(keys_a.stream_d2l, keys_b.stream_d2l);
    assert_eq!(keys_a.stream_l2d, keys_b.stream_l2d);
    assert_eq!(keys_a.audit_d2l, keys_b.audit_d2l);
    assert_eq!(keys_a.audit_l2d, keys_b.audit_l2d);
    assert_eq!(keys_a.handoff, keys_b.handoff);
}

#[test]
fn different_handshake_hash_produces_different_keys() {
    let keys_a = derive_all_keys(&test_handshake_hash());
    let keys_b = derive_all_keys(&other_handshake_hash());
    assert_ne!(keys_a.envelope_d2l, keys_b.envelope_d2l);
    assert_ne!(keys_a.stream_d2l, keys_b.stream_d2l);
    assert_ne!(keys_a.handoff, keys_b.handoff);
}

#[test]
fn envelope_keys_differ_by_direction() {
    let keys = derive_all_keys(&test_handshake_hash());
    assert_ne!(keys.envelope_d2l, keys.envelope_l2d);
}

#[test]
fn header_keys_differ_by_direction() {
    let keys = derive_all_keys(&test_handshake_hash());
    assert_ne!(keys.header_d2l, keys.header_l2d);
}

#[test]
fn stream_keys_differ_by_direction() {
    let keys = derive_all_keys(&test_handshake_hash());
    assert_ne!(keys.stream_d2l, keys.stream_l2d);
}

#[test]
fn audit_keys_differ_by_direction() {
    let keys = derive_all_keys(&test_handshake_hash());
    assert_ne!(keys.audit_d2l, keys.audit_l2d);
}
