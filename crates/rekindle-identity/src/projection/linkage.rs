//! LinkageProof — bidirectional binding between pseudonym and global root.
//!
//! Published at the user's discretion. Both keys sign domain-separated
//! statements over the pair; the wire object carries both signatures.
//! One-directional claims are forgeable in one direction and are rejected
//! at parse (D-07).

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::locator::GovernanceKey;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::origin::tags::derivation_tags;
use crate::projection::pseudonym::Pseudonym;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

/// Bidirectional proof linking a `Pseudonym` to an `IdentityRoot`.
///
/// Carries two signatures:
/// - `sig_by_root`: the global root signs (root, pseudonym, governance, epoch, hlc)
///   under LINKAGE_ROOT_SIGN domain.
/// - `sig_by_pseudonym`: the pseudonym key signs (pseudonym, root, governance, epoch, hlc)
///   under LINKAGE_PSEUDONYM_SIGN domain.
///
/// Both must verify for the proof to be valid. A proof with only one
/// valid signature is rejected (prevents reputation theft and false
/// attribution attacks).
#[derive(Debug, Clone)]
pub struct LinkageProof {
    pub root: IdentityRoot,
    pub epoch: RotationEpoch,
    pub pseudonym: Pseudonym,
    pub governance: GovernanceKey,
    pub issued_at: Hlc,
    pub sig_by_root: Signature64,
    pub sig_by_pseudonym: Signature64,
}

// ── Signable forms (two directions, two domain tags) ────────────

struct RootSignsLinkage {
    root_bytes: [u8; 32],
    pseudonym_bytes: [u8; 32],
    governance_canonical: Vec<u8>,
    epoch: u64,
    issued_at: Hlc,
}

impl Signable for RootSignsLinkage {
    const SIGN_DOMAIN: &'static str = derivation_tags::LINKAGE_ROOT_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 5);
        encode::bytes(buf, &self.root_bytes);
        encode::bytes(buf, &self.pseudonym_bytes);
        encode::bytes(buf, &self.governance_canonical);
        encode::unsigned(buf, self.epoch);
        self.issued_at.encode_into(buf);
    }
}

struct PseudonymSignsLinkage {
    pseudonym_bytes: [u8; 32],
    root_bytes: [u8; 32],
    governance_canonical: Vec<u8>,
    epoch: u64,
    issued_at: Hlc,
}

impl Signable for PseudonymSignsLinkage {
    const SIGN_DOMAIN: &'static str = derivation_tags::LINKAGE_PSEUDONYM_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        // Note: field order is (pseudonym, root) — opposite of root direction.
        // This prevents cross-domain signature confusion.
        encode::array_head(buf, 5);
        encode::bytes(buf, &self.pseudonym_bytes);
        encode::bytes(buf, &self.root_bytes);
        encode::bytes(buf, &self.governance_canonical);
        encode::unsigned(buf, self.epoch);
        self.issued_at.encode_into(buf);
    }
}

impl LinkageProof {
    /// Create a bidirectional linkage proof.
    ///
    /// Requires both the global identity seed (for root signature) and
    /// the pseudonym seed (for pseudonym signature). The caller must
    /// already have derived the pseudonym from `derive_persona()`.
    pub fn create(
        identity_seed: &[u8; 32],
        pseudonym_seed: &[u8; 32],
        root: IdentityRoot,
        epoch: RotationEpoch,
        pseudonym: Pseudonym,
        governance: &GovernanceKey,
        issued_at: Hlc,
    ) -> Result<Self, IdentityError> {
        let gov_canonical = governance.canonical_bytes().to_vec();

        // Root signs
        let root_kp = sign::keypair_from_seed(identity_seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("linkage root sign: {e}"),
            })?;
        let root_signable = RootSignsLinkage {
            root_bytes: *root.as_bytes(),
            pseudonym_bytes: *pseudonym.as_bytes(),
            governance_canonical: gov_canonical.clone(),
            epoch: epoch.0,
            issued_at,
        };
        let sig_root = sign::sign_raw(&root_kp, &root_signable.signable_bytes());

        // Pseudonym signs
        let pseudo_kp = sign::keypair_from_seed(pseudonym_seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("linkage pseudonym sign: {e}"),
            })?;
        let pseudo_signable = PseudonymSignsLinkage {
            pseudonym_bytes: *pseudonym.as_bytes(),
            root_bytes: *root.as_bytes(),
            governance_canonical: gov_canonical,
            epoch: epoch.0,
            issued_at,
        };
        let sig_pseudo = sign::sign_raw(&pseudo_kp, &pseudo_signable.signable_bytes());

        Ok(Self {
            root,
            epoch,
            pseudonym,
            governance: governance.clone(),
            issued_at,
            sig_by_root: Signature64::from_bytes(sig_root),
            sig_by_pseudonym: Signature64::from_bytes(sig_pseudo),
        })
    }

    /// Verify both signatures. BOTH must pass — a proof with only one
    /// valid signature is rejected.
    pub fn verify(&self) -> Result<(), IdentityError> {
        let gov_canonical = self.governance.canonical_bytes().to_vec();

        // Verify root direction
        let root_signable = RootSignsLinkage {
            root_bytes: *self.root.as_bytes(),
            pseudonym_bytes: *self.pseudonym.as_bytes(),
            governance_canonical: gov_canonical.clone(),
            epoch: self.epoch.0,
            issued_at: self.issued_at,
        };
        sign::verify_raw(
            self.root.as_bytes(),
            &root_signable.signable_bytes(),
            self.sig_by_root.as_bytes(),
        ).map_err(|_| IdentityError::LinkageInvalid {
            reason: "root signature invalid".into(),
        })?;

        // Verify pseudonym direction
        let pseudo_signable = PseudonymSignsLinkage {
            pseudonym_bytes: *self.pseudonym.as_bytes(),
            root_bytes: *self.root.as_bytes(),
            governance_canonical: gov_canonical,
            epoch: self.epoch.0,
            issued_at: self.issued_at,
        };
        sign::verify_raw(
            self.pseudonym.as_bytes(),
            &pseudo_signable.signable_bytes(),
            self.sig_by_pseudonym.as_bytes(),
        ).map_err(|_| IdentityError::LinkageInvalid {
            reason: "pseudonym signature invalid".into(),
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::seed::OriginSeed;
    use crate::projection::pseudonym::derive_persona;
    use zeroize::Zeroizing;

    fn make_linkage() -> (LinkageProof, IdentityRoot) {
        let seed_bytes = [0x01u8; 32];
        let seed = OriginSeed::from_vault_bytes(Zeroizing::new(seed_bytes));
        let o = crate::origin::originate::originate_from_seed(seed).unwrap();

        let gov = GovernanceKey::parse("VLD0:testcommunity").unwrap();
        let seed2 = OriginSeed::from_vault_bytes(Zeroizing::new(seed_bytes));
        let (persona, secrets) = derive_persona(&seed2, &gov, 5).unwrap();

        let proof = LinkageProof::create(
            &seed_bytes,
            &secrets.pseudonym_seed,
            o.root,
            RotationEpoch::ORIGIN,
            persona.pseudonym,
            &gov,
            Hlc::now(),
        ).unwrap();

        (proof, o.root)
    }

    #[test]
    fn valid_linkage_verifies() {
        let (proof, _) = make_linkage();
        assert!(proof.verify().is_ok());
    }

    #[test]
    fn zeroed_root_signature_rejected() {
        let (mut proof, _) = make_linkage();
        proof.sig_by_root = Signature64::ZERO;
        let err = proof.verify().unwrap_err();
        match err {
            IdentityError::LinkageInvalid { reason } => {
                assert!(reason.contains("root"), "should mention root: {reason}");
            }
            other => panic!("expected LinkageInvalid, got: {other}"),
        }
    }

    #[test]
    fn zeroed_pseudonym_signature_rejected() {
        let (mut proof, _) = make_linkage();
        proof.sig_by_pseudonym = Signature64::ZERO;
        let err = proof.verify().unwrap_err();
        match err {
            IdentityError::LinkageInvalid { reason } => {
                assert!(reason.contains("pseudonym"), "should mention pseudonym: {reason}");
            }
            other => panic!("expected LinkageInvalid, got: {other}"),
        }
    }

    #[test]
    fn both_signatures_zeroed_rejected() {
        let (mut proof, _) = make_linkage();
        proof.sig_by_root = Signature64::ZERO;
        proof.sig_by_pseudonym = Signature64::ZERO;
        assert!(proof.verify().is_err());
    }

    #[test]
    fn cross_direction_forgery_rejected() {
        // Attacker has their own identity. They try to create a linkage
        // proof claiming a victim's pseudonym belongs to them.
        // They can sign the root direction (their own root signs the
        // victim's pseudonym). They CANNOT sign the pseudonym direction
        // (they don't have the victim's pseudonym seed).
        let attacker_seed = [0xAA; 32];
        let attacker = crate::origin::originate::originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(attacker_seed))
        ).unwrap();

        let victim_seed = [0xBB; 32];
        let gov = GovernanceKey::parse("VLD0:community").unwrap();
        let victim_origin_seed = OriginSeed::from_vault_bytes(Zeroizing::new(victim_seed));
        let (victim_persona, _) = derive_persona(&victim_origin_seed, &gov, 0).unwrap();

        // Attacker signs root direction with their key
        let attacker_kp = sign::keypair_from_seed(&attacker_seed).unwrap();
        let root_signable = RootSignsLinkage {
            root_bytes: *attacker.root.as_bytes(),
            pseudonym_bytes: *victim_persona.pseudonym.as_bytes(),
            governance_canonical: gov.canonical_bytes().to_vec(),
            epoch: 0,
            issued_at: Hlc::now(),
        };
        let sig_root = sign::sign_raw(&attacker_kp, &root_signable.signable_bytes());

        // Attacker fakes pseudonym direction with their own key (wrong key)
        let pseudo_signable = PseudonymSignsLinkage {
            pseudonym_bytes: *victim_persona.pseudonym.as_bytes(),
            root_bytes: *attacker.root.as_bytes(),
            governance_canonical: gov.canonical_bytes().to_vec(),
            epoch: 0,
            issued_at: Hlc::now(),
        };
        let sig_pseudo = sign::sign_raw(&attacker_kp, &pseudo_signable.signable_bytes());

        let forged = LinkageProof {
            root: attacker.root,
            epoch: RotationEpoch::ORIGIN,
            pseudonym: victim_persona.pseudonym,
            governance: gov,
            issued_at: Hlc::now(),
            sig_by_root: Signature64::from_bytes(sig_root),
            sig_by_pseudonym: Signature64::from_bytes(sig_pseudo),
        };

        // Root signature verifies (attacker signed it legitimately)
        // BUT pseudonym signature fails (signed by attacker key, verified
        // against victim's pseudonym key)
        assert!(forged.verify().is_err(),
            "cross-direction forgery must be rejected — attacker cannot sign as victim's pseudonym");
    }

    #[test]
    fn tampered_governance_rejected() {
        let (mut proof, _) = make_linkage();
        proof.governance = GovernanceKey::parse("VLD0:different").unwrap();
        assert!(proof.verify().is_err());
    }

    #[test]
    fn tampered_epoch_rejected() {
        let (mut proof, _) = make_linkage();
        proof.epoch = RotationEpoch(999);
        assert!(proof.verify().is_err());
    }
}
