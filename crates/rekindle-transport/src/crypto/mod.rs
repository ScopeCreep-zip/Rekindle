//! Cryptographic operations for transport-layer authentication and encryption.
//!
//! This module handles envelope signing/verification, DM crypto delegation
//! to `rekindle-secrets`, voice packet encryption, and MEK resolution.

pub mod dm_envelope;
pub mod envelope;
pub mod mek;
pub mod prekeys;
/// Re-exported from `rekindle-crypto` — the single implementation shared
/// by both tracks. The transport-local copy was a wire-compatible fork
/// kept in sync by hand; deleted in favor of this re-export.
pub use rekindle_crypto::group::pseudonym;
pub mod signal_session;
pub mod signal_store;
pub mod voice_crypto;
