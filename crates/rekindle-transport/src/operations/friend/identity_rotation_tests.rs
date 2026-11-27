use super::*;

#[test]
fn a_rotation_verifies_only_for_its_recipient_and_both_signers() {
    let (old, new, bob, carol) = (
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
    );
    let to_bob = rotation(&old, &new, &bob.public_key_bytes());
    assert!(verified(
        &old.public_key_bytes(),
        &bob.public_key_bytes(),
        &to_bob
    ));
    // Replayed to Carol, or claimed by someone else.
    assert!(!verified(
        &old.public_key_bytes(),
        &carol.public_key_bytes(),
        &to_bob
    ));
    assert!(!verified(
        &carol.public_key_bytes(),
        &bob.public_key_bytes(),
        &to_bob
    ));
    // A forged new-side signature.
    let mut forged = to_bob.clone();
    forged.sig_new = carol.sign(&[0]).to_bytes();
    assert!(!verified(
        &old.public_key_bytes(),
        &bob.public_key_bytes(),
        &forged
    ));
}
