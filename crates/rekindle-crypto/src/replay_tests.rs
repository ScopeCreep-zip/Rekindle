use super::*;

#[test]
fn the_message_id_binds_the_ratchet_key_and_the_counter() {
    let id = ratchet_message_id(&[1; 32], 7);
    assert_eq!(id, ratchet_message_id(&[1; 32], 7));
    assert_ne!(id, ratchet_message_id(&[2; 32], 7));
    assert_ne!(id, ratchet_message_id(&[1; 32], 8));
    assert_ne!(id, replay_id(&[1; 32], &[0; 16]));
}

#[test]
fn the_id_binds_the_sender_and_the_nonce() {
    let id = replay_id(&[1; 32], &[2; 16]);
    assert_eq!(id, replay_id(&[1; 32], &[2; 16]));
    assert_ne!(id, replay_id(&[3; 32], &[2; 16]));
    assert_ne!(id, replay_id(&[1; 32], &[4; 16]));
    // Under its own context, not a bare hash of the same bytes.
    let mut input = [1u8; 48];
    input[32..].copy_from_slice(&[2; 16]);
    assert_ne!(id, *blake3::hash(&input).as_bytes());
}
