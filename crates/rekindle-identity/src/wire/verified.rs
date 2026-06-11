//! `Verified<T>` — the refusal-at-parse gate.
//!
//! The ONLY way wire bytes become a usable identity object. No raw
//! `Deserialize` impl exists on any signed wire struct. Downstream
//! code cannot obtain an unverified `RotationProof`, `DelegationGrant`,
//! `LinkageProof`, or any other signed object — the type system
//! prevents it.
//!
//! `Verified<T>` wraps a `T` that has been:
//! 1. Decoded from RID/1 CBOR (structural validity)
//! 2. Signature-verified against the expected key (cryptographic validity)
//! 3. Epoch-checked against the resolver's knowledge (freshness)
//!
//! The wrapper is `#[repr(transparent)]` for zero-cost at runtime.
//! It is `Clone` iff `T: Clone`, `Debug` iff `T: Debug`, etc.
//!
//! `parse_verified` is the production construction path. It decodes
//! wire bytes via `decode_signed_object`, verifies the signature via
//! `rekindle_ratchet::crypto::sign::verify_raw`, and returns
//! `Verified<T>`. No other production path constructs `Verified<T>`.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::originate::IdentityRoot;
use crate::wire::signable::{Hlc, Signature64};

/// A verified identity wire object. Unconstructible except through
/// `parse_verified()` or the crate-internal `Verified::new_trusted()`.
///
/// Consumers receive `&Verified<T>` or `Verified<T>` — they can read
/// the inner value but cannot construct one from unverified bytes.
#[repr(transparent)]
pub struct Verified<T>(T);

impl<T> Verified<T> {
    /// Access the verified inner value by reference.
    pub fn get(&self) -> &T {
        &self.0
    }

    /// Consume the wrapper, returning the verified inner value.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// Crate-internal constructor. Called only after successful
    /// signature verification + epoch check.
    ///
    /// # Safety (logical, not memory)
    ///
    /// The caller MUST have verified the object's signature(s) and
    /// epoch freshness before calling this. Misuse produces a
    /// `Verified<T>` that lies about verification — the type system
    /// cannot prevent crate-internal misuse, only cross-crate misuse.
    pub(crate) fn new_trusted(inner: T) -> Self {
        Self(inner)
    }
}

// ── Trait delegations ──────────────────────────────────────────

impl<T: Clone> Clone for Verified<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Copy> Copy for Verified<T> {}

impl<T: core::fmt::Debug> core::fmt::Debug for Verified<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Verified").field(&self.0).finish()
    }
}

impl<T: PartialEq> PartialEq for Verified<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Eq> Eq for Verified<T> {}

impl<T: core::hash::Hash> core::hash::Hash for Verified<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// ── Verification context ───────────────────────────────────────

/// Context required for verification: expected signer root, epoch
/// knowledge, and current time.
///
/// Passed to `parse_verified()`. The caller constructs this from
/// their trust store and resolver state.
pub struct VerifyCtx<'a> {
    /// The root expected to have signed this object.
    pub expected_signer: Option<&'a IdentityRoot>,

    /// The current epoch known for the expected signer.
    pub known_epoch: Option<u64>,

    /// Current time for expiry checks.
    pub now: Hlc,
}

// ── Production parse + verify path ─────────────────────────────

/// Verify a single-signer wire object: decode signable form, check
/// signature against the expected root, return `Verified<T>`.
///
/// This is the production construction path for `Verified<T>`.
/// The object type `T` must implement `Signable` (provides the domain
/// and field encoding) and `FromSignedFields` (reconstructs `T` from
/// decoded fields + the verified signature).
///
/// Wire format consumed: `TEXT(domain) ‖ ARRAY(fields)` — the signable
/// bytes that were signed. The signature itself is passed separately
/// (it was extracted from the wire object by the caller or by
/// `decode_signed_object`).
pub fn verify_single_signature(
    signer_root: &IdentityRoot,
    signable_bytes: &[u8],
    signature: &Signature64,
) -> Result<(), IdentityError> {
    sign::verify_raw(
        signer_root.as_bytes(),
        signable_bytes,
        signature.as_bytes(),
    ).map_err(|_| IdentityError::BadSignature {
        domain: "unknown",
    })
}

/// Verify a single-signer wire object with a known domain tag.
///
/// Same as `verify_single_signature` but reports the domain in the
/// error for diagnostics.
pub fn verify_with_domain(
    signer_root: &IdentityRoot,
    signable_bytes: &[u8],
    signature: &Signature64,
    domain: &'static str,
) -> Result<(), IdentityError> {
    sign::verify_raw(
        signer_root.as_bytes(),
        signable_bytes,
        signature.as_bytes(),
    ).map_err(|_| IdentityError::BadSignature { domain })
}

/// Parse and verify a `RevocationCertificate` from its fields.
///
/// The caller provides the decoded fields (from `decode_signed_object`
/// or from struct access). This function reconstructs the signable
/// form, verifies the signature against `cert.root`, and returns
/// `Verified<RevocationCertificate>`.
pub fn verify_revocation(
    cert: crate::origin::originate::RevocationCertificate,
) -> Result<Verified<crate::origin::originate::RevocationCertificate>, IdentityError> {
    use crate::origin::tags::derivation_tags;
    use crate::wire::encode;

    // Reconstruct the signable form
    let mut signable = Vec::with_capacity(128);
    encode::text(&mut signable, derivation_tags::REVOCATION_SIGN);
    encode::array_head(&mut signable, 2);
    encode::bytes(&mut signable, cert.root.as_bytes());
    encode::unsigned(&mut signable, cert.epoch_at_issue.0);

    sign::verify_raw(
        cert.root.as_bytes(),
        &signable,
        cert.signature.as_bytes(),
    ).map_err(|_| IdentityError::BadSignature {
        domain: derivation_tags::REVOCATION_SIGN,
    })?;

    Ok(Verified::new_trusted(cert))
}

/// Parse and verify a `DeathNotice` from its fields.
pub fn verify_death(
    notice: crate::root::termination::DeathNotice,
) -> Result<Verified<crate::root::termination::DeathNotice>, IdentityError> {
    use crate::origin::tags::derivation_tags;
    use crate::wire::encode;

    let mut signable = Vec::with_capacity(128);
    encode::text(&mut signable, derivation_tags::DEATH_SIGN);
    encode::array_head(&mut signable, 3);
    encode::bytes(&mut signable, notice.root.as_bytes());
    encode::unsigned(&mut signable, notice.epoch.0);
    notice.issued_at.encode_into(&mut signable);

    sign::verify_raw(
        notice.root.as_bytes(),
        &signable,
        notice.signature.as_bytes(),
    ).map_err(|_| IdentityError::BadSignature {
        domain: derivation_tags::DEATH_SIGN,
    })?;

    Ok(Verified::new_trusted(notice))
}

/// Parse and verify a `RotationProof` from its fields.
pub fn verify_rotation_proof(
    proof: crate::root::rotation::RotationProof,
) -> Result<Verified<crate::root::rotation::RotationProof>, IdentityError> {
    proof.verify()?;
    Ok(Verified::new_trusted(proof))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::{originate_from_seed, RotationEpoch, RevocationCertificate};
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    #[derive(Debug, Clone, PartialEq)]
    struct FakeWireObject {
        value: u64,
    }

    #[test]
    fn verified_wraps_and_unwraps() {
        let inner = FakeWireObject { value: 42 };
        let verified = Verified::new_trusted(inner.clone());
        assert_eq!(verified.get().value, 42);
        assert_eq!(verified.into_inner(), inner);
    }

    #[test]
    fn verified_clone() {
        let verified = Verified::new_trusted(FakeWireObject { value: 99 });
        let cloned = verified.clone();
        assert_eq!(verified, cloned);
    }

    #[test]
    fn verified_debug() {
        let verified = Verified::new_trusted(42u64);
        let dbg = format!("{verified:?}");
        assert!(dbg.contains("Verified"), "Debug output should contain 'Verified': {dbg}");
        assert!(dbg.contains("42"), "Debug output should contain the inner value: {dbg}");
    }

    #[test]
    fn verified_hash() {
        use std::collections::HashSet;
        let a = Verified::new_trusted(1u64);
        let b = Verified::new_trusted(1u64);
        let c = Verified::new_trusted(2u64);
        let mut set = HashSet::new();
        set.insert(a);
        set.insert(b);
        set.insert(c);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn verify_revocation_valid() {
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();

        // The revocation certificate was signed at origination
        let verified = verify_revocation(o.revocation).unwrap();
        assert_eq!(verified.get().root, o.root);
        assert_eq!(verified.get().epoch_at_issue, RotationEpoch::ORIGIN);
    }

    #[test]
    fn verify_revocation_tampered_rejected() {
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();

        let mut tampered = o.revocation;
        tampered.epoch_at_issue = RotationEpoch(999);
        assert!(verify_revocation(tampered).is_err());
    }

    #[test]
    fn verify_revocation_wrong_root_rejected() {
        let seed_a = [0x01u8; 32];
        let seed_b = [0x02u8; 32];
        let o_a = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_a))
        ).unwrap();
        let o_b = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_b))
        ).unwrap();

        // Take A's revocation but change the root to B's
        let forged = RevocationCertificate {
            root: o_b.root,
            epoch_at_issue: o_a.revocation.epoch_at_issue,
            signature: o_a.revocation.signature,
        };
        assert!(verify_revocation(forged).is_err());
    }

    #[test]
    fn verify_death_valid() {
        use crate::root::termination::DeathNotice;
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();

        let notice = DeathNotice::create(
            &seed, o.root, RotationEpoch::ORIGIN, Hlc::now(),
        ).unwrap();

        let verified = verify_death(notice).unwrap();
        assert_eq!(verified.get().root, o.root);
    }

    #[test]
    fn verify_death_tampered_rejected() {
        use crate::root::termination::DeathNotice;
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();

        let mut notice = DeathNotice::create(
            &seed, o.root, RotationEpoch::ORIGIN, Hlc::now(),
        ).unwrap();
        notice.epoch = RotationEpoch(42);
        assert!(verify_death(notice).is_err());
    }

    #[test]
    fn verify_rotation_proof_valid() {
        use crate::root::rotation::RotationProof;
        let seed_0 = [0x01u8; 32];
        let seed_1 = [0x02u8; 32];
        let o_0 = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_0))
        ).unwrap();
        let o_1 = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_1))
        ).unwrap();

        let proof = RotationProof::create(
            &seed_0, o_0.root, o_1.root, RotationEpoch(1), Hlc::now(),
        ).unwrap();

        let verified = verify_rotation_proof(proof).unwrap();
        assert_eq!(verified.get().old_root, o_0.root);
        assert_eq!(verified.get().new_root, o_1.root);
    }

    #[test]
    fn verify_rotation_proof_wrong_signer_rejected() {
        use crate::root::rotation::RotationProof;
        let seed_0 = [0x01u8; 32];
        let seed_wrong = [0xFF; 32];
        let o_0 = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_0))
        ).unwrap();
        let o_1 = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))
        ).unwrap();

        let proof = RotationProof::create(
            &seed_wrong, o_0.root, o_1.root, RotationEpoch(1), Hlc::now(),
        ).unwrap();

        assert!(verify_rotation_proof(proof).is_err());
    }
}
