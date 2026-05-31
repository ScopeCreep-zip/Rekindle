//! Pure CRDT merge engine for Rekindle v2.0 flat governance.
//!
//! **NO I/O. NO async. NO side effects.**
//!
//! Takes `GovernanceEntry` variants from all member subkeys, sorts by
//! `(lamport, author_pseudonym)`, and applies deterministic merge rules
//! to produce a `GovernanceState`. Every peer running the same merge on
//! the same entries produces an identical result — this is the CRDT
//! convergence guarantee.
//!
//! Tier 6 in the module hierarchy — depends only on `rekindle-types`.
//!
//! See architecture doc §4.4 for merge rules.
//! See rekindle-architecture-v2.md §4.4 for field specifications.

pub mod invite_quota;
pub mod merge;
pub mod permissions;
pub mod raid_detection;
pub mod state;
pub mod validate;

pub use raid_detection::{observe_join, resolve_thresholds, RaidAlert};
