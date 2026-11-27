//! Lamport logical clock utilities for deterministic gossip ordering.
//! The clock itself lives in `rekindle_types::lamport` (tier 1), so the
//! pure governance merge can share it.

pub use rekindle_types::lamport::{LamportClock, MAX_LAMPORT_DRIFT};

/// Property-based tests for Lamport clock ordering convergence.
/// Proves the Chiral Network's deterministic ordering guarantee:
/// given the same set of messages, all peers arrive at the same order
/// regardless of delivery sequence.
#[cfg(test)]
#[path = "lamport_proptests.rs"]
mod proptests;
