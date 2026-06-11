//! Prekey bundle binding — epoch-bound signature over a published bundle.
//!
//! A `PrekeyBundleBinding` ties a published prekey bundle to the current
//! root + epoch with freshness. Bundles whose binding fails verification
//! against the resolver's current `(root, epoch)` — or whose `issued_at`
//! predates the last observed rotation — are rejected at parse.
//!
//! This is what makes rotation actually invalidate stale bundles: the
//! bundle itself (constructed by rekindle-chat) is hashed, and the hash
//! is signed with the identity root at the current epoch. A post-rotation
//! bundle with a pre-rotation binding signature is unverifiable.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

/// Binds a published prekey bundle to the current identity root + epoch.
#[derive(Debug, Clone)]
pub struct PrekeyBundleBinding {
    /// The root that signed this binding.
    pub root: IdentityRoot,
    /// The epoch of the root at signing time.
    pub epoch: RotationEpoch,
    /// BLAKE3 hash of the bundle's canonical bytes.
    pub bundle_hash: [u8; 32],
    /// When the binding was issued.
    pub issued_at: Hlc,
    /// Signature by `root` over the PREKEY_BINDING_SIGN domain.
    pub signature: Signature64,
}

struct PrekeyBindingSignable {
    root_bytes: [u8; 32],
    epoch: u64,
    bundle_hash: [u8; 32],
    issued_at: Hlc,
}

impl Signable for PrekeyBindingSignable {
    const SIGN_DOMAIN: &'static str = derivation_tags::PREKEY_BINDING_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 4);
        encode::bytes(buf, &self.root_bytes);
        encode::unsigned(buf, self.epoch);
        encode::bytes(buf, &self.bundle_hash);
        self.issued_at.encode_into(buf);
    }
}

impl PrekeyBundleBinding {
    /// Create and sign a binding for a published bundle.
    ///
    /// `bundle_bytes`: the canonical bytes of the prekey bundle.
    /// The BLAKE3 hash is computed here — the caller passes the raw bytes,
    /// not a pre-computed hash, to prevent hash/content divergence.
    pub fn create(
        seed: &[u8; 32],
        root: IdentityRoot,
        epoch: RotationEpoch,
        bundle_bytes: &[u8],
        issued_at: Hlc,
    ) -> Result<Self, IdentityError> {
        let kp = sign::keypair_from_seed(seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("prekey binding sign: {e}"),
            })?;

        let bundle_hash = *blake3::hash(bundle_bytes).as_bytes();

        let signable = PrekeyBindingSignable {
            root_bytes: *root.as_bytes(),
            epoch: epoch.0,
            bundle_hash,
            issued_at,
        };

        let sig = sign::sign_raw(&kp, &signable.signable_bytes());

        Ok(Self {
            root,
            epoch,
            bundle_hash,
            issued_at,
            signature: Signature64::from_bytes(sig),
        })
    }

    /// Verify the binding signature against the root.
    pub fn verify(&self) -> Result<(), IdentityError> {
        let signable = PrekeyBindingSignable {
            root_bytes: *self.root.as_bytes(),
            epoch: self.epoch.0,
            bundle_hash: self.bundle_hash,
            issued_at: self.issued_at,
        };

        sign::verify_raw(
            self.root.as_bytes(),
            &signable.signable_bytes(),
            self.signature.as_bytes(),
        ).map_err(|_| IdentityError::BadSignature {
            domain: derivation_tags::PREKEY_BINDING_SIGN,
        })
    }

    /// Check that the binding's epoch is not stale relative to
    /// the locally known current epoch for this root.
    pub fn check_freshness(
        &self,
        current_epoch: RotationEpoch,
    ) -> Result<(), IdentityError> {
        if self.epoch < current_epoch {
            return Err(IdentityError::StaleBundle);
        }
        Ok(())
    }

    /// Verify the bundle hash matches the provided bundle bytes.
    pub fn verify_bundle_hash(&self, bundle_bytes: &[u8]) -> Result<(), IdentityError> {
        let computed = *blake3::hash(bundle_bytes).as_bytes();
        if computed != self.bundle_hash {
            return Err(IdentityError::BundleHashMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn test_identity() -> (IdentityRoot, [u8; 32]) {
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();
        (o.root, seed)
    }

    #[test]
    fn create_and_verify() {
        let (root, seed) = test_identity();
        let bundle = b"fake prekey bundle content";

        let binding = PrekeyBundleBinding::create(
            &seed, root, RotationEpoch::ORIGIN, bundle, Hlc::now(),
        ).unwrap();

        assert!(binding.verify().is_ok());
        assert!(binding.verify_bundle_hash(bundle).is_ok());
    }

    #[test]
    fn tampered_bundle_hash_rejected() {
        let (root, seed) = test_identity();
        let bundle = b"original bundle";

        let binding = PrekeyBundleBinding::create(
            &seed, root, RotationEpoch::ORIGIN, bundle, Hlc::now(),
        ).unwrap();

        assert!(binding.verify_bundle_hash(b"tampered bundle").is_err());
    }

    #[test]
    fn wrong_signer_rejected() {
        let (root, _seed) = test_identity();
        let wrong_seed = [0xFF; 32];
        let bundle = b"bundle";

        let binding = PrekeyBundleBinding::create(
            &wrong_seed, root, RotationEpoch::ORIGIN, bundle, Hlc::now(),
        ).unwrap();

        assert!(binding.verify().is_err());
    }

    #[test]
    fn stale_epoch_rejected() {
        let (root, seed) = test_identity();
        let bundle = b"bundle";

        let binding = PrekeyBundleBinding::create(
            &seed, root, RotationEpoch(2), bundle, Hlc::now(),
        ).unwrap();

        // Current epoch is 3 — binding at epoch 2 is stale
        assert!(binding.check_freshness(RotationEpoch(3)).is_err());
        // Current epoch is 2 — binding at epoch 2 is current
        assert!(binding.check_freshness(RotationEpoch(2)).is_ok());
        // Current epoch is 1 — binding at epoch 2 is ahead (acceptable)
        assert!(binding.check_freshness(RotationEpoch(1)).is_ok());
    }

    #[test]
    fn tampered_epoch_in_binding_rejected() {
        let (root, seed) = test_identity();
        let bundle = b"bundle";

        let mut binding = PrekeyBundleBinding::create(
            &seed, root, RotationEpoch::ORIGIN, bundle, Hlc::now(),
        ).unwrap();

        binding.epoch = RotationEpoch(999);
        assert!(binding.verify().is_err());
    }

    #[test]
    fn bundle_hash_deterministic() {
        let bundle = b"consistent content";
        let hash_a = *blake3::hash(bundle).as_bytes();
        let hash_b = *blake3::hash(bundle).as_bytes();
        assert_eq!(hash_a, hash_b);
    }
}
