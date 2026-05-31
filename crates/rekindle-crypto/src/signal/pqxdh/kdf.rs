//! HKDF-SHA256 KDF for PQXDH root-key derivation.
//!
//! Input order per the PQXDH spec (signal.org/docs/specifications/pqxdh/):
//!   `F || DH1 || DH2 || DH3 [|| DH4] || SS`
//!
//! where `F = 0xFF × 32` (the X25519 / X3DH convention — 32 bytes for
//! Curve25519). Salt is zero-filled (default HKDF salt for SHA-256).
//! Info string is `"rekindle-pqxdh-root-v1"` for domain separation from
//! any other Rekindle KDF.

use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::SharedSecret;
use zeroize::Zeroizing;

use super::PqxdhError;

const F_BYTES: [u8; 32] = [0xFF; 32];
const INFO: &[u8] = b"rekindle-pqxdh-root-v1";

/// Derive a 32-byte root key from the four DH outputs (DH4 optional) and
/// the ML-KEM shared secret. Returns a zeroizing buffer.
pub fn derive_root_key(
    dh1: &SharedSecret,
    dh2: &SharedSecret,
    dh3: &SharedSecret,
    dh4: Option<&SharedSecret>,
    ml_kem_ss: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, PqxdhError> {
    let mut ikm = Vec::with_capacity(32 * 5 + 32);
    ikm.extend_from_slice(&F_BYTES);
    ikm.extend_from_slice(dh1.as_bytes());
    ikm.extend_from_slice(dh2.as_bytes());
    ikm.extend_from_slice(dh3.as_bytes());
    if let Some(dh4) = dh4 {
        ikm.extend_from_slice(dh4.as_bytes());
    }
    ikm.extend_from_slice(ml_kem_ss);

    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(INFO, &mut *out)
        .map_err(|e| PqxdhError::Hkdf(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    use x25519_dalek::{PublicKey, StaticSecret};

    /// Build a deterministic-feeling test fixture (still random per run —
    /// fixture is built once and reused across all asserts in one test).
    fn fixture() -> (
        SharedSecret,
        SharedSecret,
        SharedSecret,
        SharedSecret,
        [u8; 32],
    ) {
        let sk = || StaticSecret::random_from_rng(OsRng);
        let (sk_a, sk_b) = (sk(), sk());
        let (sk_c, sk_d) = (sk(), sk());
        let (sk_e, sk_f) = (sk(), sk());
        let (sk_g, sk_h) = (sk(), sk());
        let dh1 = sk_a.diffie_hellman(&PublicKey::from(&sk_b));
        let dh2 = sk_c.diffie_hellman(&PublicKey::from(&sk_d));
        let dh3 = sk_e.diffie_hellman(&PublicKey::from(&sk_f));
        let dh4 = sk_g.diffie_hellman(&PublicKey::from(&sk_h));
        let ss = [7u8; 32];
        (dh1, dh2, dh3, dh4, ss)
    }

    #[test]
    fn deterministic_for_same_inputs() {
        let (dh1, dh2, dh3, dh4, ss) = fixture();
        let r1 = derive_root_key(&dh1, &dh2, &dh3, Some(&dh4), &ss).unwrap();
        let r2 = derive_root_key(&dh1, &dh2, &dh3, Some(&dh4), &ss).unwrap();
        assert_eq!(*r1, *r2, "HKDF must be deterministic for identical inputs");
    }

    #[test]
    fn dh4_changes_output() {
        let (dh1, dh2, dh3, dh4, ss) = fixture();
        let with_dh4 = derive_root_key(&dh1, &dh2, &dh3, Some(&dh4), &ss).unwrap();
        let without_dh4 = derive_root_key(&dh1, &dh2, &dh3, None, &ss).unwrap();
        assert_ne!(
            *with_dh4, *without_dh4,
            "presence of DH4 must affect root_key"
        );
    }

    #[test]
    fn ss_changes_output() {
        let (dh1, dh2, dh3, dh4, ss) = fixture();
        let mut ss2 = ss;
        ss2[0] ^= 0xFF;
        let r1 = derive_root_key(&dh1, &dh2, &dh3, Some(&dh4), &ss).unwrap();
        let r2 = derive_root_key(&dh1, &dh2, &dh3, Some(&dh4), &ss2).unwrap();
        assert_ne!(
            *r1, *r2,
            "different ML-KEM shared secret must change root_key"
        );
    }

    #[test]
    fn output_is_32_bytes() {
        let (dh1, dh2, dh3, dh4, ss) = fixture();
        let out = derive_root_key(&dh1, &dh2, &dh3, Some(&dh4), &ss).unwrap();
        assert_eq!(out.len(), 32);
    }
}
