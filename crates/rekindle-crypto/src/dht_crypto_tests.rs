use super::*;

#[test]
fn deterministic_account_key_derivation() {
    let secret = [42u8; 32];
    let key1 = DhtRecordKey::derive_account_key(&secret);
    let key2 = DhtRecordKey::derive_account_key(&secret);
    assert_eq!(key1.key, key2.key);

    // Different secret → different key
    let other = [43u8; 32];
    let key3 = DhtRecordKey::derive_account_key(&other);
    assert_ne!(key1.key, key3.key);
}

#[test]
fn encrypt_decrypt_round_trip() {
    let key = DhtRecordKey::derive_account_key(&[7u8; 32]);
    let plaintext = b"hello rekindle DHT";

    let ciphertext = key.encrypt(plaintext).unwrap();
    assert_ne!(&ciphertext[NONCE_LEN..], plaintext);

    let decrypted = key.decrypt(&ciphertext).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn wrong_key_rejection() {
    let key1 = DhtRecordKey::derive_account_key(&[1u8; 32]);
    let key2 = DhtRecordKey::derive_account_key(&[2u8; 32]);

    let ciphertext = key1.encrypt(b"secret data").unwrap();
    assert!(key2.decrypt(&ciphertext).is_err());
}

#[test]
fn short_ciphertext_rejected() {
    let key = DhtRecordKey::derive_account_key(&[1u8; 32]);
    assert!(key.decrypt(&[0u8; 10]).is_err());
}
