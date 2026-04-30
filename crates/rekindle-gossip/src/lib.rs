//! Transport-agnostic gossip mesh primitives for Rekindle v2.0.
//!
//! Tier 5 utility crate: peer selection, deduplication, Lamport clocks,
//! sender-side rate limiting, and generic broadcast helpers.

pub mod broadcast;
pub mod dedup;
pub mod lamport;
pub mod mesh;
pub mod rate_limit;
