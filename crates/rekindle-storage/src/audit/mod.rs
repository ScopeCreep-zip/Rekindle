//! BLAKE3 hash-chained audit log.
//!
//! Append-only JSONL. Each entry includes a keyed BLAKE3 hash of
//! `previous_hash || event_type || timestamp || detail`. The chain
//! can be verified independently to detect tampering or truncation.

pub mod hash_chain;

pub use hash_chain::AuditLog;
