//! Pseudonym derivation — G3, G4, G5.
//!
//! `derive_persona()` is the ONLY pseudonym producer in the workspace.
//! It derives a community-scoped identity from `OriginSeed` + `GovernanceKey`:
//!
//! - G3: pseudonym seed = blake3::derive_key(PSEUDONYM_SEED, seed ‖ substrate_byte ‖ canonical_bytes)
//! - G4: Ed25519 pseudonym keypair from pseudonym seed
//! - G5: X25519 community DH key from pseudonym seed (same DH_FROM_SEED tag as G2)

use zeroize::Zeroizing;

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::locator::GovernanceKey;
use crate::operational::dh::DhKey;
use crate::origin::originate::DhSeed;
use crate::origin::seed::OriginSeed;
use crate::origin::tags::derivation_tags;

/// Ed25519 public key scoped to a community.
///
/// Derived, not portable. Unlinkable to the `IdentityRoot` absent
/// a `LinkageProof`. Sealed newtype — prevents accidental use of
/// an `IdentityRoot` where a `Pseudonym` is expected.
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Pseudonym(pub [u8; 32]);

impl Pseudonym {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(s).map_err(|_| IdentityError::PseudonymDerivation {
            reason: "invalid hex".into(),
        })?;
        if bytes.len() != 32 {
            return Err(IdentityError::PseudonymDerivation {
                reason: format!("expected 32 bytes, got {}", bytes.len()),
            });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl core::fmt::Debug for Pseudonym {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let h = self.to_hex();
        write!(f, "Pseudonym({}…{})", &h[..8], &h[56..])
    }
}

/// Wire object a community sees. NO global material (D-08, I-L5).
///
/// Serializable — this is what gossip payloads, registry entries,
/// and community member lists carry. The `IdentityRoot` of the
/// projecting peer is NOT present in this structure.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommunityPersona {
    pub pseudonym: Pseudonym,
    pub community_dh: DhKey,
    pub slot_index: u32,
    pub governance: GovernanceKey,
}

/// Local-only resolution record. NOT serializable.
///
/// Carries the back-reference to the owning identity. This type
/// does NOT implement `Serialize` or `Deserialize` — it exists only
/// in memory, in the `PeerResolver` cache. The absence of serde
/// impls is the structural enforcement of D-08.
#[derive(Debug, Clone)]
pub struct ResolvedPersona {
    pub persona: CommunityPersona,
    pub principal: crate::root::PeerRef,
}

/// Secret material produced by `derive_persona()`.
///
/// The caller (rekindle-chat) uses the pseudonym seed for signing
/// community-scoped messages and the community DH seed for MEK wrapping.
pub struct PersonaSecrets {
    pub pseudonym_seed: Zeroizing<[u8; 32]>,
    pub community_dh_seed: DhSeed,
}

impl core::fmt::Debug for PersonaSecrets {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PersonaSecrets(REDACTED)")
    }
}

/// G3 + G4 + G5 in one call. The ONLY pseudonym producer in the workspace.
///
/// Derivation grammar (pinned):
/// - G3: `pseudonym_seed = blake3::derive_key(PSEUDONYM_SEED, seed ‖ substrate_byte ‖ canonical_bytes)`
///   - IKM is variable-length: 32 (seed) + 1 (substrate) + N (canonical).
///     Collision-safe because the seed prefix is fixed-width 32 bytes.
/// - G4: Ed25519 pseudonym keypair from `pseudonym_seed` via `keypair_from_seed`.
/// - G5: X25519 community DH seed from `pseudonym_seed` via `DH_FROM_SEED` tag.
pub fn derive_persona(
    seed: &OriginSeed,
    governance: &GovernanceKey,
    slot_index: u32,
) -> Result<(CommunityPersona, PersonaSecrets), IdentityError> {
    // G3: Pseudonym seed derivation
    //
    // IKM = OriginSeed (32 bytes) ‖ substrate_discriminant (1 byte) ‖ canonical_bytes (variable)
    //
    // v2 grammar: fixed context string, governance key in IKM (not interpolated
    // into the context string). Substrate discriminant byte in IKM per A-7.
    let canonical = governance.canonical_bytes();
    let substrate_byte = governance.substrate().discriminant_byte();

    let mut ikm = Vec::with_capacity(32 + 1 + canonical.len());
    ikm.extend_from_slice(seed.expose());
    ikm.push(substrate_byte);
    ikm.extend_from_slice(canonical);

    let pseudonym_seed_raw = blake3::derive_key(derivation_tags::PSEUDONYM_SEED, &ikm);

    // Zeroize the IKM (contains seed bytes in the clear)
    zeroize::Zeroize::zeroize(&mut ikm);

    let pseudonym_seed = Zeroizing::new(pseudonym_seed_raw);

    // G4: Ed25519 pseudonym keypair from pseudonym seed
    let kp = sign::keypair_from_seed(&pseudonym_seed)
        .map_err(|e| IdentityError::PseudonymDerivation {
            reason: format!("Ed25519 pseudonym keypair: {e}"),
        })?;
    let pseudonym_pub = sign::public_key_bytes(&kp);
    let pseudonym = Pseudonym(pseudonym_pub);

    // G5: X25519 community DH from pseudonym seed (same tag as G2)
    let community_dh_raw = blake3::derive_key(derivation_tags::DH_FROM_SEED, &*pseudonym_seed);
    let community_dh_seed = DhSeed::from_raw(Zeroizing::new(community_dh_raw));

    let community_dh = crate::operational::dh::dh_public_from_seed(&community_dh_seed)?;

    let persona = CommunityPersona {
        pseudonym,
        community_dh,
        slot_index,
        governance: governance.clone(),
    };

    let secrets = PersonaSecrets {
        pseudonym_seed,
        community_dh_seed,
    };

    Ok((persona, secrets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;

    fn test_seed(byte: u8) -> OriginSeed {
        OriginSeed::from_vault_bytes(Zeroizing::new([byte; 32]))
    }

    fn test_gov(key: &str) -> GovernanceKey {
        GovernanceKey::parse(key).unwrap()
    }

    #[test]
    fn derivation_deterministic() {
        let seed = test_seed(0x01);
        let gov = test_gov("VLD0:community-alpha");
        let (a, _) = derive_persona(&seed, &gov, 11).unwrap();

        let seed2 = test_seed(0x01);
        let (b, _) = derive_persona(&seed2, &gov, 11).unwrap();
        assert_eq!(a.pseudonym, b.pseudonym);
        assert_eq!(a.community_dh, b.community_dh);
    }

    #[test]
    fn different_seeds_different_pseudonyms() {
        let gov = test_gov("VLD0:community-alpha");
        let (a, _) = derive_persona(&test_seed(0x01), &gov, 0).unwrap();
        let (b, _) = derive_persona(&test_seed(0x02), &gov, 0).unwrap();
        assert_ne!(a.pseudonym, b.pseudonym);
        assert_ne!(a.community_dh, b.community_dh);
    }

    #[test]
    fn different_communities_different_pseudonyms() {
        let seed = test_seed(0x01);
        let (a, _) = derive_persona(&seed, &test_gov("VLD0:alpha"), 0).unwrap();

        let seed2 = test_seed(0x01);
        let (b, _) = derive_persona(&seed2, &test_gov("VLD0:beta"), 0).unwrap();
        assert_ne!(a.pseudonym, b.pseudonym);
    }

    #[test]
    fn substrate_discriminant_affects_derivation() {
        let seed = test_seed(0x01);
        let gov = test_gov("VLD0:testkey");

        // Manual derivation WITHOUT substrate byte
        let canonical = gov.canonical_bytes();
        let mut ikm_without = Vec::with_capacity(32 + canonical.len());
        ikm_without.extend_from_slice(seed.expose());
        ikm_without.extend_from_slice(canonical);
        let without_discriminant = blake3::derive_key(derivation_tags::PSEUDONYM_SEED, &ikm_without);

        // Our derivation WITH substrate byte
        let (persona, _) = derive_persona(&seed, &gov, 0).unwrap();

        // The pseudonym seed should differ (substrate byte changes IKM)
        let kp = sign::keypair_from_seed(&without_discriminant).unwrap();
        let pub_without = sign::public_key_bytes(&kp);
        assert_ne!(
            persona.pseudonym.as_bytes(), &pub_without,
            "substrate discriminant byte must affect pseudonym derivation"
        );
    }

    #[test]
    fn community_persona_no_root_bytes() {
        // Structural assertion: CommunityPersona serialized form must
        // not contain the IdentityRoot bytes of the projecting peer.
        let seed = test_seed(0x42);
        let o = originate_from_seed(seed).unwrap();
        let gov = test_gov("VLD0:testcommunity");

        let persona_seed = test_seed(0x42);
        let (persona, _) = derive_persona(&persona_seed, &gov, 5).unwrap();

        let json = serde_json::to_vec(&persona).unwrap();
        let root_bytes = o.root.as_bytes();

        // Sliding window scan: the 32-byte root value must not appear
        // anywhere in the serialized persona.
        for window in json.windows(32) {
            assert_ne!(
                window, root_bytes.as_slice(),
                "CommunityPersona serialization contains IdentityRoot bytes — \
                 unlinkability violation (D-08, I-L5)"
            );
        }
    }

    #[test]
    fn slot_index_does_not_affect_derivation() {
        let seed = test_seed(0x01);
        let gov = test_gov("VLD0:comm");
        let (a, _) = derive_persona(&seed, &gov, 0).unwrap();

        let seed2 = test_seed(0x01);
        let (b, _) = derive_persona(&seed2, &gov, 99).unwrap();
        assert_eq!(a.pseudonym, b.pseudonym, "slot_index must not affect pseudonym");
        assert_eq!(a.community_dh, b.community_dh, "slot_index must not affect DH key");
    }

    #[test]
    fn pseudonym_hex_roundtrip() {
        let seed = test_seed(0x01);
        let gov = test_gov("VLD0:test");
        let (persona, _) = derive_persona(&seed, &gov, 0).unwrap();
        let hex = persona.pseudonym.to_hex();
        let restored = Pseudonym::from_hex(&hex).unwrap();
        assert_eq!(persona.pseudonym, restored);
    }

    #[test]
    fn persona_secrets_debug_redacted() {
        let seed = test_seed(0x01);
        let gov = test_gov("VLD0:test");
        let (_, secrets) = derive_persona(&seed, &gov, 0).unwrap();
        let dbg = format!("{secrets:?}");
        assert_eq!(dbg, "PersonaSecrets(REDACTED)");
    }

    #[test]
    fn governance_key_canonicalization_with_and_without_prefix() {
        let seed = test_seed(0x01);
        let gov_prefixed = test_gov("VLD0:mykey");
        let gov_bare = test_gov("mykey");

        let (a, _) = derive_persona(&seed, &gov_prefixed, 0).unwrap();
        let seed2 = test_seed(0x01);
        let (b, _) = derive_persona(&seed2, &gov_bare, 0).unwrap();

        assert_eq!(a.pseudonym, b.pseudonym,
            "same canonical bytes must produce same pseudonym regardless of prefix");
    }
}
