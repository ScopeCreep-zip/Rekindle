//! Per-community pseudonym derivation — façade over `rekindle_secrets::derive`.
//!
//! The single implementation lives in `rekindle-secrets` (the Tier-2
//! security boundary); this module re-exports it for the many
//! `rekindle_crypto::group::pseudonym::*` call sites. It used to carry a
//! wire-compatible copy of the same HKDF derivation, kept in sync by hand
//! with the secrets and transport copies — the tests below stay as a
//! regression guard that the derivation contract holds.

pub use rekindle_secrets::derive::{
    derive_community_pseudonym, pseudonym_to_x25519, sign_with_pseudonym,
};

#[cfg(test)]
#[path = "pseudonym_tests.rs"]
mod tests;
