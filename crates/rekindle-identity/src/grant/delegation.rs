//! DelegationGrant — first-class signed edge between equal peers.
//!
//! Authority transfer is a signed wire object. Messages sent under
//! delegated authority are signed by the delegate's key and carry
//! (or reference by hash) the grant chain; the delegator's key never
//! signs the message. Audit trails always show which key actually acted.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::locator::GovernanceKey;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

use super::capability::CapabilitySet;

/// Maximum delegation chain depth (spec constant, single digit).
pub const GRANT_CHAIN_MAX_DEPTH: u8 = 4;

/// The scope a grant applies to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GrantScope {
    /// Applies globally across all communities.
    Global,
    /// Applies to a specific community.
    Community(GovernanceKey),
    /// Applies to a specific channel within a community.
    /// The `[u8; 16]` is the channel UUID bytes — bookkeeping, not identity.
    Channel {
        governance: GovernanceKey,
        channel: [u8; 16],
    },
}

impl GrantScope {
    /// Check if `child` scope is equal to or narrower than `self`.
    /// Narrowing is permitted; widening is not.
    pub fn permits(&self, child: &GrantScope) -> bool {
        match (self, child) {
            (GrantScope::Global, _) => true,
            (GrantScope::Community(a), GrantScope::Community(b)) => a == b,
            (GrantScope::Community(a), GrantScope::Channel { governance, .. }) => a == governance,
            (GrantScope::Channel { governance: ga, channel: ca },
             GrantScope::Channel { governance: gb, channel: cb }) => ga == gb && ca == cb,
            _ => false,
        }
    }

    /// Encode for the wire signable form.
    pub fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            GrantScope::Global => {
                encode::unsigned(buf, 0);
            }
            GrantScope::Community(gov) => {
                encode::array_head(buf, 2);
                encode::unsigned(buf, 1);
                encode::bytes(buf, gov.canonical_bytes());
            }
            GrantScope::Channel { governance, channel } => {
                encode::array_head(buf, 3);
                encode::unsigned(buf, 2);
                encode::bytes(buf, governance.canonical_bytes());
                encode::bytes(buf, channel);
            }
        }
    }
}

/// A signed delegation grant from one peer to another.
///
/// `grant_epoch` is monotonic per (delegator, delegate) ordered pair.
/// Latest epoch wins. A revocation is a grant at a higher epoch with
/// an empty `CapabilitySet`. This subsumes nonces, TTL-renewal
/// treadmills, and heartbeats (R-04).
#[derive(Debug, Clone)]
pub struct DelegationGrant {
    pub delegator: IdentityRoot,
    pub delegator_epoch: RotationEpoch,
    pub delegate: IdentityRoot,
    pub capabilities: CapabilitySet,
    pub scope: GrantScope,
    /// Monotonic per (delegator, delegate) edge. Latest wins.
    pub grant_epoch: u64,
    /// Optional hard expiry bound (defense in depth).
    pub not_after: Option<Hlc>,
    pub issued_at: Hlc,
    /// Signature by `delegator` over the GRANT_SIGN domain.
    pub signature: Signature64,
}

struct GrantSignable<'a> {
    delegator_bytes: [u8; 32],
    delegator_epoch: u64,
    delegate_bytes: [u8; 32],
    capabilities: &'a CapabilitySet,
    scope: &'a GrantScope,
    grant_epoch: u64,
    not_after: Option<Hlc>,
    issued_at: Hlc,
}

impl Signable for GrantSignable<'_> {
    const SIGN_DOMAIN: &'static str = derivation_tags::GRANT_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 8);
        encode::bytes(buf, &self.delegator_bytes);
        encode::unsigned(buf, self.delegator_epoch);
        encode::bytes(buf, &self.delegate_bytes);
        self.capabilities.encode_into(buf);
        self.scope.encode_into(buf);
        encode::unsigned(buf, self.grant_epoch);
        match &self.not_after {
            None => encode::null(buf),
            Some(hlc) => hlc.encode_into(buf),
        }
        self.issued_at.encode_into(buf);
    }
}

impl DelegationGrant {
    /// Create and sign a delegation grant.
    pub fn create(
        delegator_seed: &[u8; 32],
        delegator: IdentityRoot,
        delegator_epoch: RotationEpoch,
        delegate: IdentityRoot,
        capabilities: CapabilitySet,
        scope: GrantScope,
        grant_epoch: u64,
        not_after: Option<Hlc>,
        issued_at: Hlc,
    ) -> Result<Self, IdentityError> {
        let kp = sign::keypair_from_seed(delegator_seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("grant sign: {e}"),
            })?;

        let signable = GrantSignable {
            delegator_bytes: *delegator.as_bytes(),
            delegator_epoch: delegator_epoch.0,
            delegate_bytes: *delegate.as_bytes(),
            capabilities: &capabilities,
            scope: &scope,
            grant_epoch,
            not_after,
            issued_at,
        };

        let sig = sign::sign_raw(&kp, &signable.signable_bytes());

        Ok(Self {
            delegator,
            delegator_epoch,
            delegate,
            capabilities,
            scope,
            grant_epoch,
            not_after,
            issued_at,
            signature: Signature64::from_bytes(sig),
        })
    }

    /// Verify the grant's signature.
    pub fn verify_signature(&self) -> Result<(), IdentityError> {
        let signable = GrantSignable {
            delegator_bytes: *self.delegator.as_bytes(),
            delegator_epoch: self.delegator_epoch.0,
            delegate_bytes: *self.delegate.as_bytes(),
            capabilities: &self.capabilities,
            scope: &self.scope,
            grant_epoch: self.grant_epoch,
            not_after: self.not_after,
            issued_at: self.issued_at,
        };

        sign::verify_raw(
            self.delegator.as_bytes(),
            &signable.signable_bytes(),
            self.signature.as_bytes(),
        ).map_err(|_| IdentityError::BadSignature {
            domain: derivation_tags::GRANT_SIGN,
        })
    }

    /// Check if the grant is a revocation (empty capability set at a higher epoch).
    pub fn is_revocation(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Check if the grant has expired.
    pub fn is_expired(&self, now: &Hlc) -> bool {
        self.not_after.is_some_and(|deadline| now > &deadline)
    }

    /// BLAKE3 hash of the grant's signable bytes — used as a compact
    /// chain-by-reference handle in message payloads.
    pub fn reference_hash(&self) -> [u8; 32] {
        let signable = GrantSignable {
            delegator_bytes: *self.delegator.as_bytes(),
            delegator_epoch: self.delegator_epoch.0,
            delegate_bytes: *self.delegate.as_bytes(),
            capabilities: &self.capabilities,
            scope: &self.scope,
            grant_epoch: self.grant_epoch,
            not_after: self.not_after,
            issued_at: self.issued_at,
        };
        *blake3::hash(&signable.signable_bytes()).as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grant::capability::Capability;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn make_grant() -> (DelegationGrant, IdentityRoot, IdentityRoot) {
        let seed_a = [0x01u8; 32];
        let seed_b = [0x02u8; 32];
        let a = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new(seed_a))).unwrap();
        let b = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new(seed_b))).unwrap();

        let grant = DelegationGrant::create(
            &seed_a, a.root, RotationEpoch::ORIGIN,
            b.root,
            CapabilitySet::from_iter([Capability::SendMessages, Capability::ReadMessages]),
            GrantScope::Global,
            1, None, Hlc::now(),
        ).unwrap();

        (grant, a.root, b.root)
    }

    #[test]
    fn create_and_verify() {
        let (grant, _, _) = make_grant();
        assert!(grant.verify_signature().is_ok());
    }

    #[test]
    fn wrong_signer_rejected() {
        let seed_a = [0x01u8; 32];
        let seed_wrong = [0xFF; 32];
        let a = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new(seed_a))).unwrap();
        let b = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))).unwrap();

        let grant = DelegationGrant::create(
            &seed_wrong, a.root, RotationEpoch::ORIGIN,
            b.root, CapabilitySet::empty(), GrantScope::Global,
            1, None, Hlc::now(),
        ).unwrap();

        assert!(grant.verify_signature().is_err());
    }

    #[test]
    fn tampered_delegate_rejected() {
        let (mut grant, _, _) = make_grant();
        let fake = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0xFF; 32]))
        ).unwrap();
        grant.delegate = fake.root;
        assert!(grant.verify_signature().is_err());
    }

    #[test]
    fn empty_set_is_revocation() {
        let seed_a = [0x01u8; 32];
        let a = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new(seed_a))).unwrap();
        let b = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))).unwrap();

        let revoke = DelegationGrant::create(
            &seed_a, a.root, RotationEpoch::ORIGIN,
            b.root, CapabilitySet::empty(), GrantScope::Global,
            5, None, Hlc::now(),
        ).unwrap();

        assert!(revoke.is_revocation());
    }

    #[test]
    fn expiry_check() {
        let (grant, _, _) = make_grant();
        assert!(!grant.is_expired(&Hlc::now())); // no deadline

        let seed_a = [0x01u8; 32];
        let a = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new(seed_a))).unwrap();
        let b = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))).unwrap();
        let past = Hlc::new(1, 0); // epoch timestamp 1ns
        let grant_expired = DelegationGrant::create(
            &seed_a, a.root, RotationEpoch::ORIGIN,
            b.root, CapabilitySet::from_iter([Capability::ReadMessages]),
            GrantScope::Global, 1, Some(past), Hlc::now(),
        ).unwrap();
        assert!(grant_expired.is_expired(&Hlc::now()));
    }

    #[test]
    fn scope_permits_narrowing() {
        let gov = GovernanceKey::parse("VLD0:test").unwrap();
        assert!(GrantScope::Global.permits(&GrantScope::Community(gov.clone())));
        assert!(GrantScope::Community(gov.clone()).permits(&GrantScope::Channel {
            governance: gov.clone(), channel: [0; 16]
        }));
        assert!(!GrantScope::Community(gov.clone()).permits(&GrantScope::Global));
    }

    #[test]
    fn reference_hash_deterministic() {
        let (grant, _, _) = make_grant();
        let h1 = grant.reference_hash();
        let h2 = grant.reference_hash();
        assert_eq!(h1, h2);
    }
}
