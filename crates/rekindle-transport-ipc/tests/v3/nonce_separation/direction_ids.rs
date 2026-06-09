use rekindle_transport_ipc::v3::wire::constants::{
    DIRECTION_ID_D2L, DIRECTION_ID_L2D, AEAD_NONCE_LEN,
};

#[test]
fn direction_ids_are_distinct() {
    assert_ne!(
        DIRECTION_ID_D2L, DIRECTION_ID_L2D,
        "Direction identifiers MUST differ. Identical values cause \
         nonce reuse across directions — catastrophic AES-GCM \
         key-stream XOR recovery."
    );
}

#[test]
fn direction_ids_are_4_bytes() {
    assert_eq!(DIRECTION_ID_D2L.len(), 4);
    assert_eq!(DIRECTION_ID_L2D.len(), 4);
}

#[test]
fn direction_id_plus_u64_counter_equals_nonce_len() {
    let nonce_len = DIRECTION_ID_D2L.len() + std::mem::size_of::<u64>();
    assert_eq!(
        nonce_len, AEAD_NONCE_LEN,
        "direction_id (4 bytes) + u64 counter (8 bytes) must equal \
         AEAD_NONCE_LEN ({AEAD_NONCE_LEN} bytes)"
    );
}

#[test]
fn d2l_is_zero() {
    assert_eq!(DIRECTION_ID_D2L, [0, 0, 0, 0]);
}

#[test]
fn l2d_is_one() {
    assert_eq!(DIRECTION_ID_L2D, [0, 0, 0, 1]);
}
