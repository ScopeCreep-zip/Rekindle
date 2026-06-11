//! `SigningKeypair` — opaque Ed25519 keypair wrapper.
//!
//! Consumers of `SelfIdentity::signing_keypair()` receive this type.
//! It wraps `aws_lc_rs::signature::Ed25519KeyPair` but does not expose
//! it — the aws-lc-rs type never appears in the public API. This means:
//!
//! - aws-lc-rs version bumps do not cascade to consumers
//! - Adding a second crypto backend (ring, rustcrypto) changes this
//!   module's internals, not its API
//! - All signing operations are methods on the keypair, not free functions
//!   requiring a module import

use rekindle_ratchet::crypto::sign;

/// Opaque Ed25519 signing keypair.
///
/// Constructed by `SelfIdentity::signing_keypair()`. The inner type
/// is not public — consumers call methods on this struct, never
/// interact with aws-lc-rs types directly.
///
/// The keypair is reconstructed from the seed on every call to
/// `signing_keypair()` and should be dropped promptly after use.
pub struct SigningKeypair(sign::Ed25519KeyPair);

impl SigningKeypair {
    /// Crate-internal constructor from the ratchet crate's keypair.
    pub(crate) fn from_inner(inner: sign::Ed25519KeyPair) -> Self {
        Self(inner)
    }

    /// Construct from a 32-byte seed. Used for pseudonym keypair derivation
    /// where the seed comes from `SelfIdentity::pseudonym_seed()`.
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, crate::error::IdentityError> {
        let inner = sign::keypair_from_seed(seed)
            .map_err(|e| crate::error::IdentityError::KeypairDerivation {
                reason: format!("SigningKeypair::from_seed: {e}"),
            })?;
        Ok(Self(inner))
    }

    /// Extract the 32-byte Ed25519 public key.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        sign::public_key_bytes(&self.0)
    }

    /// Sign arbitrary bytes. Returns a 64-byte Ed25519 signature.
    ///
    /// The caller is responsible for domain separation — typically via
    /// the `Signable` trait's CBOR Sequence framing.
    pub fn sign_raw(&self, message: &[u8]) -> [u8; 64] {
        sign::sign_raw(&self.0, message)
    }

    /// Sign an X25519 prekey: `sign(0x01 || key_bytes)`.
    ///
    /// Used in PQXDH bundle construction for signed prekeys (SPK)
    /// and one-time prekeys (OPK).
    pub fn sign_ec_prekey(&self, key_bytes: &[u8]) -> [u8; 64] {
        sign::sign_ec_prekey(&self.0, key_bytes)
    }

    /// Sign a PQ prekey: `sign(0x02 || domain_tag || key_bytes)`.
    ///
    /// `domain_tag` is `DOMAIN_OT` for one-time or `DOMAIN_LR` for
    /// last-resort ML-KEM-768 prekeys.
    pub fn sign_pq_prekey(&self, domain_tag: &[u8], key_bytes: &[u8]) -> [u8; 64] {
        sign::sign_pq_prekey(&self.0, domain_tag, key_bytes)
    }
}

impl core::fmt::Debug for SigningKeypair {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let pub_hex = hex::encode(&self.public_key_bytes()[..4]);
        write!(f, "SigningKeypair({pub_hex}…)")
    }
}

/// Algorithm-byte prefix for X25519 keys (PQXDH rev 2, F3).
pub const ALG_X25519: u8 = sign::ALG_X25519;
/// Algorithm-byte prefix for ML-KEM-768 keys (PQXDH rev 2, F3).
pub const ALG_MLKEM768: u8 = sign::ALG_MLKEM768;
/// Domain tag for one-time PQ prekeys (F4).
pub const DOMAIN_OT: &[u8] = sign::DOMAIN_OT;
/// Domain tag for last-resort PQ prekeys (F4).
pub const DOMAIN_LR: &[u8] = sign::DOMAIN_LR;

/// Verify an X25519 prekey signature.
pub fn verify_ec_prekey(
    public_key: &[u8; 32],
    key_bytes: &[u8],
    signature: &[u8],
) -> Result<(), crate::error::IdentityError> {
    sign::verify_ec_prekey(public_key, key_bytes, signature)
        .map_err(|_| crate::error::IdentityError::BadSignature {
            domain: "ec_prekey",
        })
}

/// Verify a PQ prekey signature with domain tag.
pub fn verify_pq_prekey(
    public_key: &[u8; 32],
    domain_tag: &[u8],
    key_bytes: &[u8],
    signature: &[u8],
) -> Result<(), crate::error::IdentityError> {
    sign::verify_pq_prekey(public_key, domain_tag, key_bytes, signature)
        .map_err(|_| crate::error::IdentityError::BadSignature {
            domain: "pq_prekey",
        })
}

/// Verify a raw Ed25519 signature.
pub fn verify_raw(
    public_key: &[u8; 32],
    message: &[u8],
    signature: &[u8; 64],
) -> Result<(), crate::error::IdentityError> {
    sign::verify_raw(public_key, message, signature)
        .map_err(|_| crate::error::IdentityError::BadSignature {
            domain: "raw",
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn make_keypair() -> SigningKeypair {
        let seed = [0x01u8; 32];
        let inner = sign::keypair_from_seed(&seed).unwrap();
        SigningKeypair::from_inner(inner)
    }

    #[test]
    fn public_key_matches_derivation() {
        let kp = make_keypair();
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        assert_eq!(kp.public_key_bytes(), *o.root.as_bytes());
    }

    #[test]
    fn sign_raw_verifiable() {
        let kp = make_keypair();
        let msg = b"test message";
        let sig = kp.sign_raw(msg);
        let pub_key = kp.public_key_bytes();
        assert!(verify_raw(&pub_key, msg, &sig).is_ok());
    }

    #[test]
    fn sign_ec_prekey_verifiable() {
        let kp = make_keypair();
        let prekey = [0xAA; 32];
        let sig = kp.sign_ec_prekey(&prekey);
        let pub_key = kp.public_key_bytes();
        assert!(verify_ec_prekey(&pub_key, &prekey, &sig).is_ok());
    }

    #[test]
    fn sign_pq_prekey_verifiable() {
        let kp = make_keypair();
        let pq_key = [0xBB; 64];
        let sig = kp.sign_pq_prekey(DOMAIN_OT, &pq_key);
        let pub_key = kp.public_key_bytes();
        assert!(verify_pq_prekey(&pub_key, DOMAIN_OT, &pq_key, &sig).is_ok());
    }

    #[test]
    fn sign_pq_prekey_wrong_domain_rejected() {
        let kp = make_keypair();
        let pq_key = [0xBB; 64];
        let sig = kp.sign_pq_prekey(DOMAIN_OT, &pq_key);
        let pub_key = kp.public_key_bytes();
        assert!(verify_pq_prekey(&pub_key, DOMAIN_LR, &pq_key, &sig).is_err());
    }

    #[test]
    fn from_seed_produces_valid_keypair() {
        let seed = [0x42u8; 32];
        let kp = SigningKeypair::from_seed(&seed).unwrap();
        let msg = b"from_seed test";
        let sig = kp.sign_raw(msg);
        let pub_key = kp.public_key_bytes();
        assert!(verify_raw(&pub_key, msg, &sig).is_ok());
    }

    #[test]
    fn from_seed_deterministic() {
        let seed = [0x42u8; 32];
        let a = SigningKeypair::from_seed(&seed).unwrap();
        let b = SigningKeypair::from_seed(&seed).unwrap();
        assert_eq!(a.public_key_bytes(), b.public_key_bytes());
    }

    #[test]
    fn from_seed_matches_from_inner() {
        let seed = [0x01u8; 32];
        let from_seed = SigningKeypair::from_seed(&seed).unwrap();
        let from_inner = make_keypair(); // also uses [0x01; 32]
        assert_eq!(from_seed.public_key_bytes(), from_inner.public_key_bytes());
    }

    #[test]
    fn debug_does_not_leak_key() {
        let kp = make_keypair();
        let dbg = format!("{kp:?}");
        assert!(dbg.starts_with("SigningKeypair("));
        assert!(dbg.contains('…'));
        assert!(dbg.len() < 30);
    }
}
