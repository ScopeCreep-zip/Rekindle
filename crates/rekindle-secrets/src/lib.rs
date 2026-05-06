//! Cryptographic security boundary for Rekindle v2.0.
//!
//! This is the **sole crate** that handles raw key material. Every secret
//! type implements `Zeroize + ZeroizeOnDrop`. No other crate in the workspace
//! should import `ed25519-dalek`, `x25519-dalek`, `aes-gcm`, or `hkdf` directly.
//!
//! Tier 2 in the module hierarchy — depends only on `rekindle-types`.

pub mod derive;
pub mod invite;
pub mod keys;
pub mod mek;
pub mod rotator;
pub mod sign;
pub mod sync_key;

// Re-export ed25519_dalek for callers that need SigningKey/VerifyingKey types
// (e.g., slot_signing_to_veilid conversion in community create/join)
pub use ed25519_dalek;
