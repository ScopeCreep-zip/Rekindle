//! Cryptographic security boundary for Rekindle v2.0.
//!
//! This is the **sole crate** that handles raw key material. Every secret
//! type implements `Zeroize + ZeroizeOnDrop`. No other crate in the workspace
//! should import `ed25519-dalek`, `x25519-dalek`, `aes-gcm`, or `hkdf` directly.
//!
//! Tier 2 in the module hierarchy — depends only on `rekindle-types`.

// The sole-security-boundary claim above is only as good as the crate
// root locking it: `#![forbid(unsafe_code)]` makes "no unsafe in the
// crate that holds raw key material" a compile error, not a convention
// one submodule happens to declare locally (`session_cache.rs` did,
// before this).
#![forbid(unsafe_code)]

pub mod channel_body;
pub mod derive;
pub mod invite;
pub mod keys;
pub mod media_sender_key;
pub mod mek;
pub mod pq_keys;
pub mod rotator;
pub mod sframe;
pub mod sign;
pub mod sync_key;

// Re-export ed25519_dalek for callers that need SigningKey/VerifyingKey types
// (e.g., slot_signing_to_veilid conversion in community create/join)
pub use ed25519_dalek;

// Re-export the error type this crate's public API returns, so consumers
// can name and match on it without depending on `rekindle-types` themselves.
pub use rekindle_types::error::CryptoError;
