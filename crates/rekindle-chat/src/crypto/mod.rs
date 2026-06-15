//! Cryptographic services for the chat layer.
//!
//! Wraps `rekindle-ratchet` primitives and manages session cache +
//! MEK cache. Identity signing operations go through `SelfIdentity`
//! from `rekindle-identity` via `PlatformIO::with_identity()`.

pub mod sessions;
pub mod envelope;
pub mod mek;
