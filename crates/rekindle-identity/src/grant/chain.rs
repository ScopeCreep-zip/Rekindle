//! ChainVerifier — attenuation, depth, epoch freshness, scope narrowing.
//!
//! Verifies a presented delegation chain delegator₀→…→delegateₙ.
//! The effective authority is the intersection of all capability sets
//! along the chain, gated by depth limit and scope compatibility.

use crate::error::IdentityError;
use crate::wire::signable::Hlc;

use super::capability::CapabilitySet;
use super::delegation::{DelegationGrant, GrantScope, GRANT_CHAIN_MAX_DEPTH};

/// Callback: given a (delegator, delegate) edge, return the highest
/// known grant_epoch for that edge. If unknown, return `None`.
///
/// Used to reject stale grants: a grant with `grant_epoch` below
/// the oracle's known value is superseded.
pub trait EdgeEpochOracle {
    fn known_epoch(&self, delegator: &[u8; 32], delegate: &[u8; 32]) -> Option<u64>;
}

/// A no-op oracle that accepts all epochs (for testing).
pub struct AcceptAllEpochs;

impl EdgeEpochOracle for AcceptAllEpochs {
    fn known_epoch(&self, _delegator: &[u8; 32], _delegate: &[u8; 32]) -> Option<u64> {
        None
    }
}

/// The result of successful chain verification.
#[derive(Debug, Clone)]
pub struct EffectiveAuthority {
    /// The intersection of all capability sets along the chain.
    pub capabilities: CapabilitySet,
    /// The narrowest scope along the chain.
    pub scope: GrantScope,
    /// The final delegate (the peer exercising authority).
    pub delegate: crate::root::IdentityRoot,
    /// Chain depth (0 = direct grant, no intermediate links).
    pub depth: u8,
}

/// Verify a presented delegation chain.
///
/// Checks:
/// 1. Each link's signature is valid against its delegator root.
/// 2. Each link's `grant_epoch` is not below the oracle's known epoch.
/// 3. Chain depth ≤ `GRANT_CHAIN_MAX_DEPTH`.
/// 4. Every non-terminal link carries `Delegate{max_depth}` with
///    remaining depth ≥ rest of chain.
/// 5. Each link's scope is equal to or narrower than the previous.
/// 6. No link is expired (if `not_after` is set).
/// 7. Effective capabilities = ∩ along the chain.
///
/// Returns the effective `CapabilitySet` + scope, or an error.
pub fn verify_chain(
    chain: &[DelegationGrant],
    now: &Hlc,
    epoch_oracle: &dyn EdgeEpochOracle,
) -> Result<EffectiveAuthority, IdentityError> {
    if chain.is_empty() {
        return Err(IdentityError::AttenuationViolation);
    }

    let depth = chain.len() - 1;
    if depth as u8 > GRANT_CHAIN_MAX_DEPTH {
        return Err(IdentityError::GrantChainTooDeep {
            max: GRANT_CHAIN_MAX_DEPTH,
        });
    }

    let mut effective_caps = chain[0].capabilities.clone();
    let mut effective_scope = chain[0].scope.clone();

    for (i, grant) in chain.iter().enumerate() {
        // 1. Signature verification
        grant.verify_signature()?;

        // 2. Epoch freshness
        if let Some(known) = epoch_oracle.known_epoch(
            grant.delegator.as_bytes(),
            grant.delegate.as_bytes(),
        ) {
            if grant.grant_epoch < known {
                return Err(IdentityError::StaleGrant {
                    presented: grant.grant_epoch,
                    known,
                });
            }
        }

        // 3. Expiry
        if grant.is_expired(now) {
            return Err(IdentityError::GrantExpired);
        }

        // 4. Chain linkage (delegate of previous = delegator of current)
        if i > 0 {
            if grant.delegator != chain[i - 1].delegate {
                return Err(IdentityError::AttenuationViolation);
            }
        }

        // 5. Scope narrowing (each link's scope must be permitted by the previous)
        if i > 0 {
            if !effective_scope.permits(&grant.scope) {
                return Err(IdentityError::GrantScopeWidening);
            }
            effective_scope = grant.scope.clone();
        }

        // 6. Intersection of capabilities
        if i > 0 {
            effective_caps = effective_caps.intersection(&grant.capabilities);
        }

        // 7. Delegation capability check for non-terminal links
        if i < chain.len() - 1 {
            let remaining_depth = (chain.len() - 1 - i) as u8;
            if !grant.capabilities.has_delegate(remaining_depth) {
                return Err(IdentityError::AttenuationViolation);
            }
        }
    }

    let last = chain.last().unwrap();

    Ok(EffectiveAuthority {
        capabilities: effective_caps,
        scope: effective_scope,
        delegate: last.delegate,
        depth: depth as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grant::capability::Capability;
    use crate::origin::originate::{originate_from_seed, RotationEpoch};
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn make_root(byte: u8) -> (crate::root::IdentityRoot, [u8; 32]) {
        let seed = [byte; 32];
        let o = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new(seed))).unwrap();
        (o.root, seed)
    }

    fn direct_grant(
        from_seed: &[u8; 32],
        from: crate::root::IdentityRoot,
        to: crate::root::IdentityRoot,
        caps: CapabilitySet,
        epoch: u64,
    ) -> DelegationGrant {
        DelegationGrant::create(
            from_seed, from, RotationEpoch::ORIGIN,
            to, caps, GrantScope::Global, epoch, None, Hlc::now(),
        ).unwrap()
    }

    #[test]
    fn single_link_chain() {
        let (a, seed_a) = make_root(0x01);
        let (b, _) = make_root(0x02);

        let grant = direct_grant(
            &seed_a, a, b,
            CapabilitySet::from_iter([Capability::SendMessages, Capability::ReadMessages]),
            1,
        );

        let result = verify_chain(&[grant], &Hlc::now(), &AcceptAllEpochs).unwrap();
        assert_eq!(result.depth, 0);
        assert!(result.capabilities.contains(&Capability::SendMessages));
        assert!(result.capabilities.contains(&Capability::ReadMessages));
        assert_eq!(result.delegate, b);
    }

    #[test]
    fn two_link_chain_attenuates() {
        let (a, seed_a) = make_root(0x01);
        let (b, seed_b) = make_root(0x02);
        let (c, _) = make_root(0x03);

        let g1 = direct_grant(
            &seed_a, a, b,
            CapabilitySet::from_iter([
                Capability::SendMessages,
                Capability::ReadMessages,
                Capability::Delegate { max_depth: 2 },
            ]),
            1,
        );
        let g2 = direct_grant(
            &seed_b, b, c,
            CapabilitySet::from_iter([
                Capability::ReadMessages,
                Capability::ManageChannels, // NOT in g1 — will be attenuated out
            ]),
            1,
        );

        let result = verify_chain(&[g1, g2], &Hlc::now(), &AcceptAllEpochs).unwrap();
        assert_eq!(result.depth, 1);
        assert!(result.capabilities.contains(&Capability::ReadMessages));
        assert!(!result.capabilities.contains(&Capability::SendMessages)); // not in g2
        assert!(!result.capabilities.contains(&Capability::ManageChannels)); // not in g1
    }

    #[test]
    fn missing_delegate_capability_rejected() {
        let (a, seed_a) = make_root(0x01);
        let (b, seed_b) = make_root(0x02);
        let (c, _) = make_root(0x03);

        // g1 does NOT have Delegate capability
        let g1 = direct_grant(
            &seed_a, a, b,
            CapabilitySet::from_iter([Capability::SendMessages]),
            1,
        );
        let g2 = direct_grant(
            &seed_b, b, c,
            CapabilitySet::from_iter([Capability::SendMessages]),
            1,
        );

        let err = verify_chain(&[g1, g2], &Hlc::now(), &AcceptAllEpochs).unwrap_err();
        assert!(matches!(err, IdentityError::AttenuationViolation));
    }

    #[test]
    fn depth_exceeded_rejected() {
        let mut roots = Vec::new();
        for byte in 0..=(GRANT_CHAIN_MAX_DEPTH + 2) {
            roots.push(make_root(byte + 1));
        }

        let mut grants = Vec::new();
        for i in 0..=(GRANT_CHAIN_MAX_DEPTH as usize + 1) {
            let (from, seed) = &roots[i];
            let (to, _) = &roots[i + 1];
            grants.push(direct_grant(
                seed, *from, *to,
                CapabilitySet::from_iter([
                    Capability::SendMessages,
                    Capability::Delegate { max_depth: GRANT_CHAIN_MAX_DEPTH },
                ]),
                1,
            ));
        }

        let err = verify_chain(&grants, &Hlc::now(), &AcceptAllEpochs).unwrap_err();
        assert!(matches!(err, IdentityError::GrantChainTooDeep { .. }));
    }

    #[test]
    fn stale_epoch_rejected() {
        let (a, seed_a) = make_root(0x01);
        let (b, _) = make_root(0x02);

        let grant = direct_grant(
            &seed_a, a, b,
            CapabilitySet::from_iter([Capability::ReadMessages]),
            3, // presented epoch 3
        );

        struct KnowsEpoch4;
        impl EdgeEpochOracle for KnowsEpoch4 {
            fn known_epoch(&self, _: &[u8; 32], _: &[u8; 32]) -> Option<u64> {
                Some(4) // known epoch 4 > presented 3
            }
        }

        let err = verify_chain(&[grant], &Hlc::now(), &KnowsEpoch4).unwrap_err();
        assert!(matches!(err, IdentityError::StaleGrant { presented: 3, known: 4 }));
    }

    #[test]
    fn expired_grant_rejected() {
        let (a, seed_a) = make_root(0x01);
        let (b, _) = make_root(0x02);

        let past_deadline = Hlc::new(1, 0);
        let grant = DelegationGrant::create(
            &seed_a, a, RotationEpoch::ORIGIN,
            b, CapabilitySet::from_iter([Capability::ReadMessages]),
            GrantScope::Global, 1, Some(past_deadline), Hlc::now(),
        ).unwrap();

        let err = verify_chain(&[grant], &Hlc::now(), &AcceptAllEpochs).unwrap_err();
        assert!(matches!(err, IdentityError::GrantExpired));
    }

    #[test]
    fn scope_widening_rejected() {
        let (a, seed_a) = make_root(0x01);
        let (b, seed_b) = make_root(0x02);
        let (c, _) = make_root(0x03);
        let gov = crate::locator::GovernanceKey::parse("VLD0:test").unwrap();

        // g1 scoped to a community
        let g1 = DelegationGrant::create(
            &seed_a, a, RotationEpoch::ORIGIN,
            b, CapabilitySet::from_iter([
                Capability::SendMessages,
                Capability::Delegate { max_depth: 2 },
            ]),
            GrantScope::Community(gov), 1, None, Hlc::now(),
        ).unwrap();

        // g2 tries to widen to global
        let g2 = DelegationGrant::create(
            &seed_b, b, RotationEpoch::ORIGIN,
            c, CapabilitySet::from_iter([Capability::SendMessages]),
            GrantScope::Global, 1, None, Hlc::now(),
        ).unwrap();

        let err = verify_chain(&[g1, g2], &Hlc::now(), &AcceptAllEpochs).unwrap_err();
        assert!(matches!(err, IdentityError::GrantScopeWidening));
    }

    #[test]
    fn empty_chain_rejected() {
        let err = verify_chain(&[], &Hlc::now(), &AcceptAllEpochs).unwrap_err();
        assert!(matches!(err, IdentityError::AttenuationViolation));
    }

    #[test]
    fn broken_chain_linkage_rejected() {
        let (a, seed_a) = make_root(0x01);
        let (b, _seed_b) = make_root(0x02);
        let (c, _) = make_root(0x03);
        let (d, seed_d) = make_root(0x04);

        // g1: a → b
        let g1 = direct_grant(
            &seed_a, a, b,
            CapabilitySet::from_iter([
                Capability::SendMessages,
                Capability::Delegate { max_depth: 2 },
            ]),
            1,
        );
        // g2: d → c (NOT b → c — chain is broken)
        let g2 = direct_grant(
            &seed_d, d, c,
            CapabilitySet::from_iter([Capability::SendMessages]),
            1,
        );

        let err = verify_chain(&[g1, g2], &Hlc::now(), &AcceptAllEpochs).unwrap_err();
        assert!(matches!(err, IdentityError::AttenuationViolation));
    }

    #[test]
    fn revocation_at_higher_epoch_supersedes() {
        let (a, seed_a) = make_root(0x01);
        let (b, _) = make_root(0x02);

        // Active grant at epoch 5
        let grant = direct_grant(
            &seed_a, a, b,
            CapabilitySet::from_iter([Capability::ReadMessages]),
            5,
        );

        // Oracle reports epoch 6 for this edge (revocation was published)
        struct KnowsRevocation;
        impl EdgeEpochOracle for KnowsRevocation {
            fn known_epoch(&self, _: &[u8; 32], _: &[u8; 32]) -> Option<u64> {
                Some(6)
            }
        }

        let err = verify_chain(&[grant], &Hlc::now(), &KnowsRevocation).unwrap_err();
        assert!(matches!(err, IdentityError::StaleGrant { presented: 5, known: 6 }));
    }
}
