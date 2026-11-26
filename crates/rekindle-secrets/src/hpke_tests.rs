use super::*;

fn h(s: &str) -> Vec<u8> {
    hex::decode(s).unwrap()
}

fn h32(s: &str) -> [u8; 32] {
    h(s).try_into().unwrap()
}

/// RFC 9180 Appendix A.2's shared inputs.
const INFO: &str = "Ode on a Grecian Urn";
const AAD: &[u8] = b"Count-0";
const PT: &[u8] = b"Beauty is truth, truth beauty";

/// RFC 9180 A.2.1 (Base, X25519 / HKDF-SHA256 / ChaCha20-Poly1305),
/// sequence number 0.
#[test]
fn rfc9180_base_vector_opens() {
    let sk_r = h32("8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb");
    assert_eq!(
        public_key(&sk_r),
        h32("4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a")
    );
    let pt = open_base(
        &sk_r,
        INFO,
        AAD,
        &h("1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a"),
        &h("1c5250d8034ec2b784ba2cfd69dbdb8af406cfe3ff938e131f0def8c8b60b4db21993c62ce81883d2dd1b51a28"),
    )
    .unwrap();
    assert_eq!(pt.as_slice(), PT);
}

/// RFC 9180 A.2.3 (Auth, same suite), sequence number 0.
#[test]
fn rfc9180_auth_vector_opens() {
    let sk_r = h32("3ca22a6d1cda1bb9480949ec5329d3bf0b080ca4c45879c95eddb55c70b80b82");
    let sk_s = h32("2def0cb58ffcf83d1062dd085c8aceca7f4c0c3fd05912d847b61f3e54121f05");
    let pk_s = h32("f0f4f9e96c54aeed3f323de8534fffd7e0577e4ce269896716bcb95643c8712b");
    assert_eq!(public_key(&sk_s), pk_s);
    let enc = h("f7674cc8cd7baa5872d1f33dbaffe3314239f6197ddf5ded1746760bfc847e0e");
    let ct = h("ab1a13c9d4f01a87ec3440dbd756e2677bd2ecf9df0ce7ed73869b98e00c09be111cb9fdf077347aeb88e61bdf");
    let pt = open_auth(&sk_r, &pk_s, INFO, AAD, &enc, &ct).unwrap();
    assert_eq!(pt.as_slice(), PT);
    // Auth binds the sender: any other claimed sender fails.
    assert!(open_auth(&sk_r, &public_key(&[9; 32]), INFO, AAD, &enc, &ct).is_err());
    // And it is not a Base-mode message.
    assert!(open_base(&sk_r, INFO, AAD, &enc, &ct).is_err());
}

#[test]
fn base_round_trip_binds_key_info_and_aad() {
    let (bob, carol) = ([2u8; 32], [3u8; 32]);
    let sealed = seal_base(&public_key(&bob), INFO, AAD, PT).unwrap();
    assert_eq!(sealed.ciphertext.len(), PT.len() + TAG_LEN);
    let open = |sk: &[u8; 32], info: &str, aad: &[u8]| {
        open_base(sk, info, aad, &sealed.enc, &sealed.ciphertext)
    };
    assert_eq!(open(&bob, INFO, AAD).unwrap().as_slice(), PT);
    assert!(open(&carol, INFO, AAD).is_err());
    assert!(open(&bob, "Ode to a Nightingale", AAD).is_err());
    assert!(open(&bob, INFO, b"Count-1").is_err());
}

#[test]
fn auth_round_trip_binds_the_sender() {
    let (alice, bob, mallory) = ([1u8; 32], [2u8; 32], [4u8; 32]);
    let sealed = seal_auth(&alice, &public_key(&bob), INFO, AAD, PT).unwrap();
    let open = |sender: &[u8; 32]| {
        open_auth(
            &bob,
            &public_key(sender),
            INFO,
            AAD,
            &sealed.enc,
            &sealed.ciphertext,
        )
    };
    assert_eq!(open(&alice).unwrap().as_slice(), PT);
    assert!(open(&mallory).is_err());
}

#[test]
fn a_malformed_or_tampered_message_fails() {
    let bob = [2u8; 32];
    let sealed = seal_base(&public_key(&bob), INFO, AAD, PT).unwrap();
    assert!(open_base(&bob, INFO, AAD, &sealed.enc[..31], &sealed.ciphertext).is_err());
    let mut ct = sealed.ciphertext.clone();
    ct[0] ^= 1;
    assert!(open_base(&bob, INFO, AAD, &sealed.enc, &ct).is_err());
}
