use super::*;
use rekindle_secrets::pq_keys::MlKemSecret;

#[test]
fn the_type_byte_leads_and_the_key_follows() {
    let x = X25519Public::from([9u8; 32]);
    assert_eq!(encode_ec(&x)[0], X25519_TYPE);
    assert_eq!(&encode_ec(&x)[1..], x.as_bytes());

    let identity = crate::Identity::generate();
    let encoded = encode_identity(&identity.public_key());
    assert_eq!(encoded[0], ED25519_TYPE);
    assert_eq!(encoded[1..], identity.public_key_bytes());

    let (_, kem) = MlKemSecret::generate();
    let encoded = encode_kem(&kem);
    assert_eq!(encoded[0], ML_KEM_768_TYPE);
    assert_eq!(&encoded[1..], kem.as_bytes());
}

#[test]
fn an_identity_and_its_montgomery_form_encode_apart() {
    let identity = crate::Identity::generate();
    assert_ne!(
        encode_identity(&identity.public_key()),
        encode_ec(&identity.to_x25519_public())
    );
}
