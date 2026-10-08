//! PreKey bundle for Signal Protocol session establishment.
//!
//! Phase 3b of the decomposed-harvest plan unified the wire-format
//! `PreKeyBundle` shape across rekindle-crypto and rekindle-transport.
//! This module re-exports `rekindle_crypto::signal::PreKeyBundle` so the
//! daemon-track transport uses the same struct as the Tauri-track Signal
//! Protocol implementation.

pub use rekindle_crypto::signal::PreKeyBundle;

use postcard;

/// Postcard helpers — replicate the legacy `PreKeyBundle::to_bytes` /
/// `from_bytes` API so existing transport callers don't need updates.
pub fn to_bytes(bundle: &PreKeyBundle) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_stdvec(bundle)
}

pub fn from_bytes(data: &[u8]) -> Result<PreKeyBundle, postcard::Error> {
    postcard::from_bytes(data)
}
