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
