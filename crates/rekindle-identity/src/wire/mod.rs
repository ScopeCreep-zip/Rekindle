//! RID/1 wire encoding layer.
//!
//! Defines the deterministic CBOR profile, encoder, decoder, `Signable`
//! trait, and `Verified<T>` refusal-at-parse wrapper. Every signed
//! identity wire object passes through this layer.
//!
//! # RID/1 Profile Restrictions
//!
//! - No maps (major 5)
//! - No tags (major 6)
//! - No floats (major 7 additional 25/26/27)
//! - No indefinite-length forms
//! - No negative integers (major 1)
//! - Simple values: only true (20), false (21), null (22)
//! - Maximum nesting depth: 4
//! - Maximum array element count: 16
//! - Trailing bytes after the top-level object: rejected
//! - Non-minimal integer encoding: rejected
//!
//! Within this type subset, RFC 8949 §4.2 Core Deterministic Encoding
//! and dCBOR produce byte-identical output. No external CBOR library
//! is needed.

pub mod encode;
pub mod decode;
pub mod signable;
pub mod verified;

// Re-exports for convenience.
pub use signable::{Hlc, Signable, Signature64};
pub use verified::{
    Verified, VerifyCtx,
    verify_single_signature, verify_with_domain,
    verify_revocation, verify_death, verify_rotation_proof,
};
