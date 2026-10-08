//! Signal Protocol storage — re-exports of the canonical traits and
//! in-memory implementations from `rekindle-crypto`.
//!
//! This file used to carry a verbatim fork of the traits and memory
//! stores, differing only in error type (`TransportError` instead of
//! `CryptoError`) — its own comment called itself "parallel to
//! `rekindle_crypto::signal::store`". One definition now serves both
//! tracks; the `SignalSessionManager` maps `CryptoError` onto
//! `TransportError` at its public boundary.

pub use rekindle_crypto::signal::memory_stores::{
    MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore,
};
pub use rekindle_crypto::signal::store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};
