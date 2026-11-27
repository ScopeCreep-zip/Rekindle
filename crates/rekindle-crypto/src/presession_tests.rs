use super::*;

const KIND: u16 = 0x0002;
const INFO: &str = domains::DM_FRIEND_REQUEST_HPKE;

#[test]
fn a_sealed_frame_verifies_and_opens_only_for_its_recipient() {
    let (alice, bob, carol) = (
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
    );
    let (a, b) = (alice.public_key_bytes(), bob.public_key_bytes());
    let sealed = seal(&alice, &b, KIND, INFO, b"hello").unwrap();
    verify(&b, &a, KIND, &sealed).unwrap();
    assert_eq!(
        open(&bob, &a, KIND, INFO, &sealed).unwrap().as_slice(),
        b"hello"
    );
    // Re-addressed, relabelled, or claimed by another sender.
    let c = carol.public_key_bytes();
    assert_eq!(verify(&c, &a, KIND, &sealed), Err(Refused::BadSignature));
    assert_eq!(verify(&b, &a, 0x0009, &sealed), Err(Refused::BadSignature));
    assert_eq!(verify(&b, &c, KIND, &sealed), Err(Refused::BadSignature));
    assert_eq!(
        open(&carol, &a, KIND, INFO, &sealed),
        Err(Refused::DoesNotOpen)
    );
    assert_eq!(
        open(&bob, &c, KIND, INFO, &sealed),
        Err(Refused::DoesNotOpen)
    );
    assert_eq!(
        open(&bob, &a, KIND, domains::DM_PREKEY_REJECTED_HPKE, &sealed),
        Err(Refused::DoesNotOpen)
    );
}

#[test]
fn a_tampered_ciphertext_fails_the_signature() {
    let (alice, bob) = (Identity::generate(), Identity::generate());
    let (a, b) = (alice.public_key_bytes(), bob.public_key_bytes());
    let mut sealed = seal(&alice, &b, KIND, INFO, b"hello").unwrap();
    sealed.ciphertext[0] ^= 1;
    assert_eq!(verify(&b, &a, KIND, &sealed), Err(Refused::BadSignature));
}
