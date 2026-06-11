//! Layer 5 — Projection. Context-scoped identity derived from global
//! identity + governance key. Unlinkable to the global root by
//! construction; linkable only via published `LinkageProof`.

pub mod pseudonym;
pub mod linkage;

pub use pseudonym::{
    Pseudonym, CommunityPersona, ResolvedPersona,
    PersonaSecrets, derive_persona,
};
pub use linkage::LinkageProof;

/// Derive a community member slot seed from a pseudonym seed, governance
/// key, and slot index. Variable data goes in the IKM, not the context string.
pub fn derive_slot_seed(
    pseudonym_seed: &[u8; 32],
    governance_key: &crate::locator::GovernanceKey,
    slot_index: u32,
) -> [u8; 32] {
    let mut ikm = Vec::with_capacity(32 + 64 + 4);
    ikm.extend_from_slice(pseudonym_seed);
    ikm.extend_from_slice(governance_key.canonical_bytes());
    ikm.extend_from_slice(&slot_index.to_le_bytes());
    blake3::derive_key(crate::origin::tags::derivation_tags::SLOT_SEED, &ikm)
}
