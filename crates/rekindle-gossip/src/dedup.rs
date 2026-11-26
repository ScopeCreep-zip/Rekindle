//! FIFO deduplication cache for gossip fan-out loops.
//!
//! The implementation is `rekindle_codec::dedup::DedupCache`. This
//! module forked it to replace the owned `(String, String, String)` key
//! with a Blake3-truncated 64-bit hash (M9.1); that optimisation has
//! been folded back into codec, whose own module doc already claimed
//! dedup as its scope, so there is one implementation again. The tests
//! below stay as a regression guard on the FIFO contract.

pub use rekindle_codec::dedup::DedupCache;

#[cfg(test)]
#[path = "dedup_tests.rs"]
mod tests;
