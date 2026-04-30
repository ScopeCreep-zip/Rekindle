//! Cryptographic operations for transport-layer authentication and encryption.
//!
//! This module handles envelope signing/verification, DM crypto delegation
//! to `rekindle-secrets`, voice packet encryption, and MEK resolution.

pub mod envelope;
pub mod dm_envelope;
pub mod voice_crypto;
pub mod mek;
