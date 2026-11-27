use super::*;

#[test]
fn encrypt_decrypt_roundtrip() {
    let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
    let secrets = b"slot_seed:abc,mek:xyz,index:42";
    let encrypted = encrypt_invite_secrets(code, secrets).unwrap();
    let decrypted = decrypt_invite_secrets(code, &encrypted).unwrap();
    assert_eq!(secrets.as_slice(), &decrypted);
}

#[test]
fn wrong_code_fails() {
    let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
    let wrong = "ffffffffffffffffffffffffffffffff";
    let encrypted = encrypt_invite_secrets(code, b"secret").unwrap();
    assert!(decrypt_invite_secrets(wrong, &encrypted).is_err());
}

#[test]
fn hash_deterministic() {
    let code = "testcode123";
    assert_eq!(hash_invite_code(code), hash_invite_code(code));
}

#[test]
fn hash_differs() {
    assert_ne!(hash_invite_code("code1"), hash_invite_code("code2"));
}

/// The id is the public half of the Ed25519 key seeded by BLAKE3
/// `derive_key` under the invite-join context: a proof by that key
/// verifies under the id, and neither the id nor a bare hash of the code
/// makes one.
#[test]
fn the_id_is_the_join_key_and_only_the_code_signs_under_it() {
    use crate::sign::verify_signature;
    let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
    let id = hash_invite_code(code);
    let seed = blake3::derive_key(domains::INVITE_JOIN, code.as_bytes());
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    assert_eq!(id, hex::encode(public));
    let sig = sign_invite_join(code, b"proof");
    assert!(verify_signature(&public, b"proof", &sig).is_ok());
    assert!(invite_join_verifies(&id, b"proof", &sig));
    assert!(!invite_join_verifies(&id, b"other", &sig));
    assert!(!invite_join_verifies("not hex", b"proof", &sig));
    assert!(verify_signature(&public, b"other", &sig).is_err());
    let by_id = sign_invite_join(&id, b"proof");
    assert!(verify_signature(&public, b"proof", &by_id).is_err());
    assert_ne!(id, hex::encode(seed));
}

/// The tag is keyed by the code, not its public lookup id: the id
/// cannot produce it, and it binds the bytes it covers.
#[test]
fn a_pre_join_read_tag_needs_the_code() {
    let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
    let tag = pre_join_read_tag(code, b"request");
    assert!(pre_join_read_tag_verifies(code, b"request", &tag));
    assert!(!pre_join_read_tag_verifies(code, b"other", &tag));
    assert!(!pre_join_read_tag_verifies(
        &hash_invite_code(code),
        b"request",
        &tag
    ));
    let id_tag = pre_join_read_tag(&hash_invite_code(code), b"request");
    assert!(!pre_join_read_tag_verifies(code, b"request", &id_tag));
}

#[test]
fn too_short_fails() {
    assert!(decrypt_invite_secrets("code", &[0u8; 11]).is_err());
}
