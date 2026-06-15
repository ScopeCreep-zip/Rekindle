//! Canonical identity types for the Rekindle platform.
//!
//! This crate is the single source of truth for identity in the entire
//! workspace. Every other crate imports identity types from here, never
//! constructs them ad-hoc from scattered strings.
//!
//! # Identity Hierarchy
//!
//! - **Layer 0 — Origin:** `OriginSeed` (private, never wire), derivation tags
//! - **Layer 1 — Root:** `IdentityRoot` (THE public identity), rotation, revocation
//! - **Layer 2 — Operational:** `DhKey`, `DeviceSigningKey`, prekey binding
//! - **Layer 3 — Locator:** `ProfileLocator`, `GovernanceKey` (routing, not identity)
//! - **Layer 4 — Persona:** `DisplayName`, `KindDescriptor` (presentation, never enforcement)
//! - **Layer 5 — Projection:** `Pseudonym`, `CommunityPersona` (per-community, unlinkable)
//!
//! # Architecture Invariants
//!
//! - No I/O. No async. No transport types. Pure types + derivation + verification.
//! - Every signed wire object passes through the `Verified<T>` refusal-at-parse gate.
//! - Vault labels use full-width hex via sealed `Label` type — no truncation, no `&str`.
//! - `IdentityRoot` is the ONLY value used for peer equality, session keying, and vault labels.
//! - Layer 3/4 values NEVER participate in KDF, label, or equality operations.

#![deny(unsafe_code)]

pub mod error;
pub mod wire;
pub mod origin;
pub mod root;
pub mod session;
pub mod operational;
pub mod locator;
pub mod persona;
pub mod projection;
pub mod grant;
pub mod trust;
pub mod vault_label;
pub mod registry;
pub mod peer;
pub mod self_id;
pub mod signing;
pub mod prelude;

// ── Top-level re-exports ────────────────────────────────────────

pub use error::IdentityError;

// Layer 0
pub use origin::{OriginSeed, OriginatedIdentity, RestoredIdentity};
pub use origin::{originate, originate_from_seed, restore};
pub use origin::tags::derivation_tags;

// Layer 1
pub use root::{IdentityRoot, PeerRef, RotationEpoch, RevocationCertificate};
pub use root::rotation::{RotationProof, RotationChain};
pub use root::termination::DeathNotice;

// Layer 2 — Operational
pub use origin::DhSeed;
pub use operational::{DhKey, dh_public_from_seed, dh_agree, x25519_seed_from, x25519_public_from_raw_seed};
pub use operational::{DeviceId, DeviceSigningKey, DeviceRecord};
pub use operational::PrekeyBundleBinding;

// Layer 3 — Locator
pub use locator::{Substrate, ProfileLocator, MailboxLocator, InboxLocator, GovernanceKey};
pub use locator::{LocatorKind, LocatorEntry, LocatorRecord};

// Layer 4 — Persona
pub use persona::{DisplayName, KindDescriptor};

// Layer 5 — Projection
pub use projection::{Pseudonym, CommunityPersona, ResolvedPersona, PersonaSecrets, derive_persona};
pub use projection::{LinkageProof, derive_slot_seed, derive_slot_keypair};

// Grant / Delegation
pub use grant::{Capability, CapabilitySet, CustomCapability};
pub use grant::{DelegationGrant, GrantScope, GRANT_CHAIN_MAX_DEPTH};
pub use grant::{verify_chain, EffectiveAuthority, EdgeEpochOracle, AcceptAllEpochs};

// Session
pub use session::{SessionAnchor, session_anchor};

// Trust
pub use trust::{TrustState, TrustEvent, TrustRecord, TrustStore, IdentityStatusChange};

// Vault Labels
pub use vault_label::Label;

// Registry
pub use registry::{ResidentIdentity, ResidentSet, PeerResolver, ResolvedPeer};

// Composites
pub use peer::{PeerId, CryptoIdentity, NetworkAddr, SocialProfile};
pub use self_id::{SelfIdentity, SelfVerificationState, OriginationResult};

// Wire
pub use wire::{Hlc, Signable, Signature64, Verified, VerifyCtx};

// Signing — opaque keypair wrapper + verify functions + constants.
// Consumers use SigningKeypair methods, never aws-lc-rs types directly.
pub use signing::{
    SigningKeypair,
    verify_ec_prekey, verify_pq_prekey, verify_raw as verify_signature,
    ALG_X25519, ALG_MLKEM768, DOMAIN_OT, DOMAIN_LR,
};
