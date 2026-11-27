use super::*;

#[test]
fn sync_key_roundtrips_per_subkey() {
    let sync_key = SyncKey::from_master_secret(b"master-identity-secret-32-bytes!");
    let plaintext = b"hello sync";
    let blob = encrypt_subkey(&sync_key, 1, plaintext).unwrap();
    let recovered = decrypt_subkey(&sync_key, 1, &blob).unwrap();
    assert_eq!(plaintext.to_vec(), recovered);
}

#[test]
fn subkey_aad_binds_index() {
    let sync_key = SyncKey::from_master_secret(b"master-identity-secret-32-bytes!");
    let blob = encrypt_subkey(&sync_key, 1, b"plaintext").unwrap();
    // Decrypting with the wrong subkey index must fail (AAD mismatch).
    let result = decrypt_subkey(&sync_key, 2, &blob);
    assert!(
        result.is_err(),
        "subkey AAD must reject cross-subkey replay"
    );
}

#[test]
fn pairing_key_wraps_and_unwraps_master_secret() {
    let salt = random_pairing_salt();
    let pairing_key = PairingKey::derive("ABCDEF123456", &salt);
    let secret = [0x42u8; 32];
    let (ct, nonce) = pairing_key.wrap_master_secret(&secret).unwrap();
    let recovered = pairing_key.unwrap_master_secret(&ct, &nonce).unwrap();
    assert_eq!(secret.to_vec(), recovered);
}

#[test]
fn pairing_key_rejects_wrong_code() {
    let salt = random_pairing_salt();
    let alice = PairingKey::derive("CORRECT-CODE", &salt);
    let bob = PairingKey::derive("WRONG-CODE", &salt);
    let (ct, nonce) = alice.wrap_master_secret(&[0x42u8; 32]).unwrap();
    let result = bob.unwrap_master_secret(&ct, &nonce);
    assert!(result.is_err(), "wrong pairing code must not unwrap");
}

#[test]
fn pairing_codes_have_expected_shape() {
    let code = generate_pairing_code();
    assert_eq!(code.len(), 12);
    assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
}
