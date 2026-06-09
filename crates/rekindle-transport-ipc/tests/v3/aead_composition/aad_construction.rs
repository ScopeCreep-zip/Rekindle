use rekindle_transport_ipc::v3::codec::aead::FrameCipher;
use rekindle_transport_ipc::v3::wire::constants::{ENVELOPE_LEN, STREAM_HEADER_LEN, DIRECTION_ID_D2L};

fn test_key() -> [u8; 32] {
    [0xEE; 32]
}

fn test_envelope_bytes() -> [u8; ENVELOPE_LEN] {
    [0x11; 32]
}

fn test_header_bytes() -> [u8; STREAM_HEADER_LEN] {
    [0x22; 32]
}

fn test_cipher() -> FrameCipher {
    FrameCipher::aes256gcm(&test_key(), DIRECTION_ID_D2L).expect("key init")
}

fn wrong_key_cipher() -> FrameCipher {
    FrameCipher::aes256gcm(&[0xFF; 32], DIRECTION_ID_D2L).expect("key init")
}

#[test]
fn aead_roundtrip_with_correct_aad() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"hello world";

    let ciphertext = cipher.seal(1, &env, Some(&hdr), plaintext);
    let recovered = cipher.open(1, &env, Some(&hdr), &ciphertext)
        .expect("AEAD must succeed with matching AAD");
    assert_eq!(&recovered, plaintext);
}

#[test]
fn aead_roundtrip_control_lane_no_header() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let plaintext = b"control frame";

    let ciphertext = cipher.seal(1, &env, None, plaintext);
    let recovered = cipher.open(1, &env, None, &ciphertext)
        .expect("AEAD must succeed for control lane");
    assert_eq!(&recovered, plaintext);
}

#[test]
fn aead_fails_with_tampered_envelope() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"hello world";

    let ciphertext = cipher.seal(1, &env, Some(&hdr), plaintext);

    let mut tampered_env = env;
    tampered_env[1] = 0xFF;
    assert!(cipher.open(1, &tampered_env, Some(&hdr), &ciphertext).is_err());
}

#[test]
fn aead_fails_with_tampered_header() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"hello world";

    let ciphertext = cipher.seal(1, &env, Some(&hdr), plaintext);

    let mut tampered_hdr = hdr;
    tampered_hdr[2] = 0xFF;
    assert!(cipher.open(1, &env, Some(&tampered_hdr), &ciphertext).is_err());
}

#[test]
fn aead_fails_with_swapped_envelope_header_order() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"hello world";

    let ciphertext = cipher.seal(1, &env, Some(&hdr), plaintext);

    assert!(
        cipher.open(1, &hdr, Some(&env), &ciphertext).is_err(),
        "AEAD must fail with swapped AAD order"
    );
}

#[test]
fn aead_fails_with_wrong_nonce() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"hello world";

    let ciphertext = cipher.seal(1, &env, Some(&hdr), plaintext);
    assert!(cipher.open(2, &env, Some(&hdr), &ciphertext).is_err());
}

#[test]
fn aead_fails_with_wrong_key() {
    let cipher = test_cipher();
    let wrong = wrong_key_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"hello world";

    let ciphertext = cipher.seal(1, &env, Some(&hdr), plaintext);
    assert!(wrong.open(1, &env, Some(&hdr), &ciphertext).is_err());
}

#[test]
fn aead_fails_with_header_present_vs_absent() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"test";

    // Seal WITH header
    let ciphertext = cipher.seal(1, &env, Some(&hdr), plaintext);
    // Open WITHOUT header — AAD mismatch
    assert!(cipher.open(1, &env, None, &ciphertext).is_err());
}

#[test]
fn aead_fails_with_header_absent_vs_present() {
    let cipher = test_cipher();
    let env = test_envelope_bytes();
    let hdr = test_header_bytes();
    let plaintext = b"test";

    // Seal WITHOUT header
    let ciphertext = cipher.seal(1, &env, None, plaintext);
    // Open WITH header — AAD mismatch
    assert!(cipher.open(1, &env, Some(&hdr), &ciphertext).is_err());
}

#[test]
fn direction_id_baked_in_at_construction() {
    let cipher = test_cipher();
    assert_eq!(cipher.direction_id(), DIRECTION_ID_D2L);
}
