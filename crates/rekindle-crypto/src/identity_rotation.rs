//! The identity-rotation signatures (plan E2.3, `IdentityRotated` 0x0108):
//! the old identity and the new one both sign
//! `IDENTITY_ROTATED ‖ old ‖ new ‖ recipient`, binding the recipient so a
//! notice cannot be replayed to someone else.

use rekindle_types::domains;

use crate::Identity;

/// What both identities sign for `recipient`.
#[must_use]
pub fn signing_bytes(old: &[u8; 32], new: &[u8; 32], recipient: &[u8; 32]) -> Vec<u8> {
    let label = domains::IDENTITY_ROTATED.as_bytes();
    let mut bytes = Vec::with_capacity(label.len() + 96);
    bytes.extend_from_slice(label);
    bytes.extend_from_slice(old);
    bytes.extend_from_slice(new);
    bytes.extend_from_slice(recipient);
    bytes
}

/// The two signatures of a rotation from `old` to `new` for `recipient`:
/// the old identity's, then the new one's.
#[must_use]
pub fn sign(old: &Identity, new: &Identity, recipient: &[u8; 32]) -> ([u8; 64], [u8; 64]) {
    let bytes = signing_bytes(&old.public_key_bytes(), &new.public_key_bytes(), recipient);
    (old.sign(&bytes).to_bytes(), new.sign(&bytes).to_bytes())
}

/// Whether a rotation from `old` to a different `new` for `recipient` is
/// signed by both identities.
#[must_use]
pub fn verify(
    old: &[u8; 32],
    new: &[u8; 32],
    recipient: &[u8; 32],
    sig_old: &[u8; 64],
    sig_new: &[u8; 64],
) -> bool {
    if old == new {
        return false;
    }
    let bytes = signing_bytes(old, new, recipient);
    let signed_by = |key: &[u8; 32], signature: &[u8; 64]| {
        ed25519_dalek::VerifyingKey::from_bytes(key).is_ok_and(|key| {
            Identity::verify(
                &key,
                &bytes,
                &ed25519_dalek::Signature::from_bytes(signature),
            )
            .is_ok()
        })
    };
    signed_by(old, sig_old) && signed_by(new, sig_new)
}

#[cfg(test)]
mod tests {
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
}
