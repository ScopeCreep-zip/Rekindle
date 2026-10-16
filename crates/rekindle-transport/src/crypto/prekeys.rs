//! PreKey bundle for Signal Protocol session establishment.
//!
//! Phase 3b of the decomposed-harvest plan unified the wire-format
//! `PreKeyBundle` shape across rekindle-crypto and rekindle-transport.
//! This module re-exports `rekindle_crypto::signal::PreKeyBundle` so the
//! daemon-track transport uses the same struct as the Tauri-track Signal
//! Protocol implementation.

pub use rekindle_crypto::signal::PreKeyBundle;
