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

/// Derive an Ed25519 signing keypair for a SMPL member slot.
///
/// Every community member knows the `slot_seed` (from community metadata
/// or invite secrets). Any member can derive any slot's keypair — this is
/// by design for the universal SMPL schema. The SMPL record's `m_key` for
/// each slot is set to the public key derived here at record creation time.
///
/// Returns a `SigningKeypair` whose `public_key_bytes()` matches the SMPL
/// member ID for this slot, and whose 64-byte serialization (pub ‖ secret)
/// is accepted by `Transport::write_record` and `Transport::open_record`
/// as the writer parameter.
pub fn derive_slot_keypair(
    slot_seed: &[u8; 32],
    slot_index: u32,
) -> Result<crate::signing::SigningKeypair, crate::error::IdentityError> {
    let mut ikm = Vec::with_capacity(32 + 4);
    ikm.extend_from_slice(slot_seed);
    ikm.extend_from_slice(&slot_index.to_le_bytes());
    let seed = blake3::derive_key(
        crate::origin::tags::derivation_tags::SLOT_KEYPAIR,
        &ikm,
    );
    crate::signing::SigningKeypair::from_seed(&seed)
}
