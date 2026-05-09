//! Cryptographic primitives wrapping `aws-lc-rs`.
//!
//! Each module exposes a minimal, purpose-specific API. No primitive
//! is used outside these wrappers — the ratchet modules call these
//! functions, never `aws-lc-rs` directly.

pub mod aead;
pub mod kdf;
pub mod kem;
pub mod dh;
pub mod sign;
