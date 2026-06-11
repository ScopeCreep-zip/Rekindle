//! Rotation proofs and chain verification.
//!
//! A `RotationProof` is a signed statement where the root at epoch *n*
//! endorses the root at epoch *n+1*. A `RotationChain` is a verified
//! sequence of contiguous proofs from a known anchor to a claimed head.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

use super::ROTATION_CHAIN_MAX;

/// A signed statement linking epoch *n* to epoch *n+1*.
///
/// Wire object. The `old_root` at `epoch - 1` signs `(new_root, epoch, issued_at)`.
#[derive(Debug, Clone)]
pub struct RotationProof {
    pub old_root: IdentityRoot,
    pub new_root: IdentityRoot,
    /// Epoch of `new_root`. MUST equal `old_root`'s epoch + 1.
    pub epoch: RotationEpoch,
    pub issued_at: Hlc,
    /// Signature by `old_root` over the ROTATION_SIGN domain signable form.
    pub signature: Signature64,
}

/// Internal signable form for rotation proofs.
struct RotationSignable {
    old_root_bytes: [u8; 32],
    new_root_bytes: [u8; 32],
    epoch: u64,
    issued_at: Hlc,
}

impl Signable for RotationSignable {
    const SIGN_DOMAIN: &'static str = derivation_tags::ROTATION_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 4);
        encode::bytes(buf, &self.old_root_bytes);
        encode::bytes(buf, &self.new_root_bytes);
        encode::unsigned(buf, self.epoch);
        self.issued_at.encode_into(buf);
    }
}

impl RotationProof {
    /// Create and sign a rotation proof. Called by the identity holder
    /// who is rotating from `old_seed` to a new identity.
    pub fn create(
        old_seed: &[u8; 32],
        old_root: IdentityRoot,
        new_root: IdentityRoot,
        new_epoch: RotationEpoch,
        issued_at: Hlc,
    ) -> Result<Self, IdentityError> {
        let kp = sign::keypair_from_seed(old_seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("rotation sign: {e}"),
            })?;

        let signable = RotationSignable {
            old_root_bytes: *old_root.as_bytes(),
            new_root_bytes: *new_root.as_bytes(),
            epoch: new_epoch.0,
            issued_at,
        };
        let sig = sign::sign_raw(&kp, &signable.signable_bytes());

        Ok(Self {
            old_root,
            new_root,
            epoch: new_epoch,
            issued_at,
            signature: Signature64::from_bytes(sig),
        })
    }

    /// Verify this proof: signature by `old_root` over the signable form.
    pub fn verify(&self) -> Result<(), IdentityError> {
        let signable = RotationSignable {
            old_root_bytes: *self.old_root.as_bytes(),
            new_root_bytes: *self.new_root.as_bytes(),
            epoch: self.epoch.0,
            issued_at: self.issued_at,
        };
        sign::verify_raw(
            self.old_root.as_bytes(),
            &signable.signable_bytes(),
            self.signature.as_bytes(),
        ).map_err(|_| IdentityError::BadSignature {
            domain: derivation_tags::ROTATION_SIGN,
        })
    }
}

/// A verified chain from a known-pinned (root, epoch) to a claimed (root, epoch).
///
/// Construction verifies: contiguity, signatures, monotone epochs,
/// max length, and anchor match.
#[derive(Debug, Clone)]
pub struct RotationChain {
    proofs: Vec<RotationProof>,
    head_root: IdentityRoot,
    head_epoch: RotationEpoch,
}

impl RotationChain {
    /// Verify a presented chain from `anchor` to the chain's head.
    ///
    /// `anchor`: the locally known (root, epoch) pair.
    /// `proofs`: the presented chain, in epoch order.
    ///
    /// Returns the verified chain, or an error on any verification failure.
    pub fn verify(
        anchor: (IdentityRoot, RotationEpoch),
        proofs: Vec<RotationProof>,
    ) -> Result<Self, IdentityError> {
        if proofs.is_empty() {
            return Ok(Self {
                proofs: Vec::new(),
                head_root: anchor.0,
                head_epoch: anchor.1,
            });
        }

        if proofs.len() > ROTATION_CHAIN_MAX {
            return Err(IdentityError::ChainTooLong { max: ROTATION_CHAIN_MAX });
        }

        // First link must chain from the anchor
        if proofs[0].old_root != anchor.0 {
            return Err(IdentityError::ChainAnchorMismatch);
        }

        let expected_first_epoch = anchor.1.next();
        if proofs[0].epoch != expected_first_epoch {
            return Err(IdentityError::BrokenChain { at: proofs[0].epoch.0 });
        }

        // Verify each link
        proofs[0].verify()?;

        for i in 1..proofs.len() {
            let prev = &proofs[i - 1];
            let curr = &proofs[i];

            // Contiguity: curr.old_root == prev.new_root
            if curr.old_root != prev.new_root {
                return Err(IdentityError::BrokenChain { at: curr.epoch.0 });
            }

            // Monotone: curr.epoch == prev.epoch + 1
            let expected_epoch = prev.epoch.next();
            if curr.epoch != expected_epoch {
                return Err(IdentityError::BrokenChain { at: curr.epoch.0 });
            }

            curr.verify()?;
        }

        let last = proofs.last().unwrap();
        Ok(Self {
            head_root: last.new_root,
            head_epoch: last.epoch,
            proofs,
        })
    }

    /// The root and epoch at the head of the verified chain.
    pub fn head(&self) -> (IdentityRoot, RotationEpoch) {
        (self.head_root, self.head_epoch)
    }

    /// Number of links in the chain.
    pub fn len(&self) -> usize {
        self.proofs.len()
    }

    /// Whether the chain is empty (anchor == head, no rotation).
    pub fn is_empty(&self) -> bool {
        self.proofs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn make_identity(seed_byte: u8) -> (IdentityRoot, [u8; 32]) {
        let seed = OriginSeed::from_vault_bytes(Zeroizing::new([seed_byte; 32]));
        let o = originate_from_seed(seed).unwrap();
        let kp = sign::keypair_from_seed(&[seed_byte; 32]).unwrap();
        let _ = kp; // just to verify seed works
        (o.root, [seed_byte; 32])
    }

    #[test]
    fn single_rotation_verifies() {
        let (root_0, seed_0) = make_identity(0x01);
        let (root_1, _seed_1) = make_identity(0x02);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1,
            RotationEpoch(1), Hlc::now(),
        ).unwrap();

        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN),
            vec![proof],
        ).unwrap();

        assert_eq!(chain.head(), (root_1, RotationEpoch(1)));
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn three_link_chain_verifies() {
        let (root_0, seed_0) = make_identity(0x10);
        let (root_1, seed_1) = make_identity(0x20);
        let (root_2, seed_2) = make_identity(0x30);
        let (root_3, _) = make_identity(0x40);

        let p1 = RotationProof::create(&seed_0, root_0, root_1, RotationEpoch(1), Hlc::now()).unwrap();
        let p2 = RotationProof::create(&seed_1, root_1, root_2, RotationEpoch(2), Hlc::now()).unwrap();
        let p3 = RotationProof::create(&seed_2, root_2, root_3, RotationEpoch(3), Hlc::now()).unwrap();

        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN),
            vec![p1, p2, p3],
        ).unwrap();

        assert_eq!(chain.head(), (root_3, RotationEpoch(3)));
        assert_eq!(chain.len(), 3);
    }

    #[test]
    fn anchor_mismatch_rejected() {
        let (root_0, seed_0) = make_identity(0x01);
        let (root_1, _) = make_identity(0x02);
        let (wrong_anchor, _) = make_identity(0xFF);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1,
            RotationEpoch(1), Hlc::now(),
        ).unwrap();

        let err = RotationChain::verify(
            (wrong_anchor, RotationEpoch::ORIGIN),
            vec![proof],
        ).unwrap_err();

        assert!(matches!(err, IdentityError::ChainAnchorMismatch));
    }

    #[test]
    fn gap_in_chain_rejected() {
        let (root_0, seed_0) = make_identity(0x01);
        let (root_1, seed_1) = make_identity(0x02);
        let (root_3, _) = make_identity(0x04);

        let p1 = RotationProof::create(&seed_0, root_0, root_1, RotationEpoch(1), Hlc::now()).unwrap();
        // Skip epoch 2, jump to 3
        let p3 = RotationProof::create(&seed_1, root_1, root_3, RotationEpoch(3), Hlc::now()).unwrap();

        let err = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN),
            vec![p1, p3],
        ).unwrap_err();

        assert!(matches!(err, IdentityError::BrokenChain { .. }));
    }

    #[test]
    fn wrong_signer_rejected() {
        let (root_0, _seed_0) = make_identity(0x01);
        let (root_1, _) = make_identity(0x02);
        let (_, wrong_seed) = make_identity(0xFF);

        // Sign with wrong key
        let proof = RotationProof::create(
            &wrong_seed, root_0, root_1,
            RotationEpoch(1), Hlc::now(),
        ).unwrap();

        let err = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN),
            vec![proof],
        ).unwrap_err();

        assert!(matches!(err, IdentityError::BadSignature { .. }));
    }

    #[test]
    fn chain_exceeds_max_length_rejected() {
        // We can't easily create 65 valid rotations in a test, but
        // we can verify the length check fires before signature checks.
        let (root_0, _) = make_identity(0x01);
        let dummy_proof = RotationProof {
            old_root: root_0,
            new_root: root_0,
            epoch: RotationEpoch(1),
            issued_at: Hlc::now(),
            signature: Signature64::ZERO,
        };
        let proofs = vec![dummy_proof; ROTATION_CHAIN_MAX + 1];

        let err = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN),
            proofs,
        ).unwrap_err();

        assert!(matches!(err, IdentityError::ChainTooLong { .. }));
    }

    #[test]
    fn empty_chain_returns_anchor() {
        let (root, _) = make_identity(0x01);
        let chain = RotationChain::verify(
            (root, RotationEpoch(5)),
            vec![],
        ).unwrap();

        assert_eq!(chain.head(), (root, RotationEpoch(5)));
        assert!(chain.is_empty());
    }

    #[test]
    fn rotation_proof_signature_verify_standalone() {
        let (root_0, seed_0) = make_identity(0x01);
        let (root_1, _) = make_identity(0x02);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1,
            RotationEpoch(1), Hlc::now(),
        ).unwrap();

        assert!(proof.verify().is_ok());

        // Tamper with new_root — signature should fail
        let mut tampered = proof.clone();
        tampered.new_root = root_0; // wrong
        assert!(tampered.verify().is_err());
    }
}
