//! Ed25519 signature verification for gossip envelopes.
//!
//! Used by the reader-validates model: every gossip message is signed by the
//! sender's pseudonym key, and every receiver verifies before processing.

use ed25519_dalek::{Signature, VerifyingKey};
use rekindle_types::error::CryptoError;

/// Verify an Ed25519 signature against a public key.
///
/// - `public_key_bytes`: The signer's Ed25519 public key (32 bytes).
/// - `data`: The signed data (typically serialized envelope bytes).
/// - `signature_bytes`: The 64-byte Ed25519 signature.
pub fn verify_signature(
    public_key_bytes: &[u8; 32],
    data: &[u8],
    signature_bytes: &[u8; 64],
) -> Result<(), CryptoError> {
    let verifying_key = VerifyingKey::from_bytes(public_key_bytes)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid Ed25519 public key: {e}")))?;
    let signature = Signature::from_bytes(signature_bytes);
    verifying_key
        .verify_strict(data, &signature)
        .map_err(|e| CryptoError::Verification(format!("signature verification failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::{derive_community_pseudonym, sign_with_pseudonym};

    #[test]
    fn sign_then_verify() {
        let key = derive_community_pseudonym(&[1u8; 32], "c");
        let data = b"hello world";
        let sig = sign_with_pseudonym(&key, data);
        let pub_bytes = key.verifying_key().to_bytes();
        assert!(verify_signature(&pub_bytes, data, &sig).is_ok());
    }

    #[test]
    fn wrong_data_fails() {
        let key = derive_community_pseudonym(&[1u8; 32], "c");
        let sig = sign_with_pseudonym(&key, b"hello");
        let pub_bytes = key.verifying_key().to_bytes();
        assert!(verify_signature(&pub_bytes, b"wrong", &sig).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let key = derive_community_pseudonym(&[1u8; 32], "c");
        let wrong_key = derive_community_pseudonym(&[2u8; 32], "c");
        let sig = sign_with_pseudonym(&key, b"hello");
        let wrong_pub = wrong_key.verifying_key().to_bytes();
        assert!(verify_signature(&wrong_pub, b"hello", &sig).is_err());
    }

    /// Behavioral spec for why this workspace uses `verify_strict` everywhere.
    ///
    /// `verify_strict` rejects two malleability classes that `verify` accepts:
    ///
    /// 1. Signatures from **small-order (a.k.a. "weak") public keys**, where
    ///    `pubkey * cofactor == identity`. With such a key, an attacker can
    ///    forge a signature that verifies against *multiple distinct messages*
    ///    — the canonical repudiation attack documented at
    ///    <https://hdevalence.ca/blog/2020-10-04-its-25519am/>.
    ///
    /// 2. Signatures whose `R` component is small-order (low-order torsion).
    ///
    /// This test mirrors `ed25519-dalek`'s own `repudiation` test (see
    /// `ed25519-dalek-2.x/tests/ed25519.rs`). We construct a public key from
    /// `EIGHT_TORSION[4]` (a small-order point), find a signature `(R, S)`
    /// such that the verification equation `[S]B == R + [k]A` is satisfied
    /// for *two different messages*, then assert:
    ///
    ///   * `verify` accepts the forged signature for both messages.
    ///   * `verify_strict` rejects the forged signature for both messages
    ///     because the public key is small-order.
    ///
    /// If this distinction ever stops holding, every Ed25519 verification
    /// site in this workspace has gained the malleability/repudiation
    /// surface — re-audit immediately.
    #[test]
    fn test_verify_strict_rejects_malleable_signatures() {
        use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
        use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
        use curve25519_dalek::scalar::Scalar;
        use curve25519_dalek::traits::IsIdentity;
        use ed25519_dalek::Verifier;
        use sha2::{Digest, Sha512};
        use std::ops::Neg;

        // EIGHT_TORSION[4] — the canonical small-order Edwards point at index 4.
        // Same bytes used by `ed25519-dalek`'s own malleability test.
        const EIGHT_TORSION_4: [u8; 32] = [
            236, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
            255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 127,
        ];
        let weak_pubkey = CompressedEdwardsY(EIGHT_TORSION_4);
        let pubkey_point: EdwardsPoint = weak_pubkey
            .decompress()
            .expect("EIGHT_TORSION[4] decompresses");

        // Build the per-signature challenge scalar
        //     k = H(R || A || M) mod L
        // (no prehash context, matching plain Ed25519).
        fn compute_challenge(message: &[u8], a: &EdwardsPoint, r: &EdwardsPoint) -> Scalar {
            let mut h = Sha512::default();
            h.update(r.compress().as_bytes());
            h.update(a.compress().as_bytes());
            h.update(message);
            Scalar::from_hash(h)
        }

        let message1: &[u8] = b"Send 100 USD to Alice";
        let message2: &[u8] = b"Send 100000 USD to Alice";

        // Loop until we find (s, R) such that the verification equation
        //     [S]B == R + [k]A
        // holds for BOTH messages simultaneously. With a small-order pubkey A,
        // [k]A is small-order regardless of k, so the equation reduces to
        // checking that (-A + k*A).is_identity() — which succeeds with high
        // probability across random k for the right A.
        let mut rng = rand::rngs::OsRng;
        let (s, r) = loop {
            let s_candidate = {
                let mut cand = Scalar::random(&mut rng);
                while cand == Scalar::ZERO {
                    cand = Scalar::random(&mut rng);
                }
                cand
            };
            let r_candidate = s_candidate * ED25519_BASEPOINT_POINT + pubkey_point.neg();
            let k1 = compute_challenge(message1, &pubkey_point, &r_candidate);
            let k2 = compute_challenge(message2, &pubkey_point, &r_candidate);
            if (pubkey_point.neg() + k1 * pubkey_point).is_identity()
                && (pubkey_point.neg() + k2 * pubkey_point).is_identity()
            {
                break (s_candidate, r_candidate);
            }
        };

        // Pack into a 64-byte Ed25519 signature.
        let mut sig_bytes = [0u8; 64];
        sig_bytes[..32].copy_from_slice(r.compress().as_bytes());
        sig_bytes[32..].copy_from_slice(s.as_bytes());
        let signature = Signature::from_bytes(&sig_bytes);
        let vk = VerifyingKey::from_bytes(weak_pubkey.as_bytes()).expect("vk decompresses");

        // The forgery: `verify` accepts the SAME signature for BOTH messages —
        // this is the repudiation/malleability attack.
        assert!(vk.is_weak(), "EIGHT_TORSION[4] must register as weak");
        assert!(
            vk.verify(message1, &signature).is_ok(),
            "verify (lax) must accept forged signature for message1 — \
             if this fails, the test's forgery construction is broken"
        );
        assert!(
            vk.verify(message2, &signature).is_ok(),
            "verify (lax) must accept the SAME forged signature for message2 — \
             this is the repudiation attack that verify_strict defends against"
        );

        // verify_strict refuses both because the public key is small-order.
        assert!(
            vk.verify_strict(message1, &signature).is_err(),
            "verify_strict MUST reject signatures from small-order public keys"
        );
        assert!(
            vk.verify_strict(message2, &signature).is_err(),
            "verify_strict MUST reject signatures from small-order public keys"
        );

        // And our high-level wrapper (which uses verify_strict) also rejects.
        let pub_bytes = vk.to_bytes();
        assert!(
            verify_signature(&pub_bytes, message1, &sig_bytes).is_err(),
            "verify_signature wrapper must inherit verify_strict's small-order rejection"
        );
        assert!(
            verify_signature(&pub_bytes, message2, &sig_bytes).is_err(),
            "verify_signature wrapper must inherit verify_strict's small-order rejection"
        );
    }
}
