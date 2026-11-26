use super::*;

#[test]
fn a_rotation_verifies_only_for_its_recipient_and_both_signers() {
    let (old, new, bob, carol) = (
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
    );
    let (o, n) = (old.public_key_bytes(), new.public_key_bytes());
    let (b, c) = (bob.public_key_bytes(), carol.public_key_bytes());
    let (sig_old, sig_new) = sign(&old, &new, &b);
    assert!(verify(&o, &n, &b, &sig_old, &sig_new));
    // Replayed to Carol, claimed by someone else, or a forged half.
    assert!(!verify(&o, &n, &c, &sig_old, &sig_new));
    assert!(!verify(&c, &n, &b, &sig_old, &sig_new));
    assert!(!verify(&o, &n, &b, &sig_old, &carol.sign(&[0]).to_bytes()));
    // A rotation to the same key is no rotation.
    let (same_old, same_new) = sign(&old, &old, &b);
    assert!(!verify(&o, &o, &b, &same_old, &same_new));
}
