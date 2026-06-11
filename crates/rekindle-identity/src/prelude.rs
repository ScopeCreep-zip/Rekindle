//! Prelude — single import for all consumers.
//!
//! `use rekindle_identity::prelude::*;` imports every type a consumer
//! typically needs. Adding a type to the identity crate updates this
//! module once — consumers don't change until they use the new type.
//!
//! This is the recommended import path. Individual imports from
//! `rekindle_identity::{module}` still work for surgical needs.

// Composites — the primary consumer-facing types
pub use crate::peer::{PeerId, CryptoIdentity, NetworkAddr, SocialProfile};
pub use crate::self_id::{SelfIdentity, SelfVerificationState, OriginationResult};

// Layer 0 — Origin
pub use crate::origin::originate::{
    IdentityRoot, RotationEpoch, OriginatedIdentity, RestoredIdentity,
};
pub use crate::origin::seed::OriginSeed;
pub use crate::origin::tags::derivation_tags;

// Layer 1 — Root
pub use crate::root::PeerRef;
pub use crate::root::rotation::{RotationProof, RotationChain};
pub use crate::root::termination::DeathNotice;
pub use crate::origin::originate::RevocationCertificate;

// Layer 2 — Operational
pub use crate::operational::{DhKey, dh_public_from_seed, dh_agree};
pub use crate::origin::originate::DhSeed;

// Layer 3 — Locator
pub use crate::locator::{ProfileLocator, MailboxLocator, InboxLocator, GovernanceKey};

// Layer 4 — Persona
pub use crate::persona::DisplayName;

// Layer 5 — Projection
pub use crate::projection::{
    Pseudonym, CommunityPersona, ResolvedPersona, PersonaSecrets, derive_persona,
};

// Session
pub use crate::session::{SessionAnchor, session_anchor};

// Trust
pub use crate::trust::{TrustState, TrustEvent, TrustRecord, TrustStore, IdentityStatusChange};

// Grant / Delegation
pub use crate::grant::{Capability, CapabilitySet, DelegationGrant, GrantScope};

// Vault Labels
pub use crate::vault_label::Label;

// Wire
pub use crate::wire::{Hlc, Signable, Signature64, Verified};

// Signing — opaque keypair + verify functions + PQXDH constants
pub use crate::signing::{
    SigningKeypair,
    verify_ec_prekey, verify_pq_prekey, verify_raw,
    ALG_X25519, ALG_MLKEM768, DOMAIN_OT, DOMAIN_LR,
};

// Error
pub use crate::error::IdentityError;
