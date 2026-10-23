//! Pure CRDT merge engine for Rekindle v2.0 flat governance.
//!
//! **NO I/O. NO async. NO side effects.**
//!
//! Takes the verified governance op set and the pinned `GenesisCert`,
//! resolves it power-first (`merge`), and produces a `GovernanceState`.
//! Every peer running the same merge on the same ops produces an identical result — this is the CRDT
//! convergence guarantee.
//!
//! Tier 6 in the module hierarchy — depends on `rekindle-types` and `rekindle-secrets` (op signatures).
//!
//! See architecture doc §4.4 for merge rules.
//! See rekindle-architecture-v2.md §4.4 for field specifications.

#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod genesis;
pub mod intent_facts;
pub mod invite_quota;
pub mod merge;
pub mod op;
pub mod permissions;
pub mod power;
pub mod raid_detection;
pub mod state;
pub mod validate;

pub use raid_detection::{observe_join, resolve_thresholds, RaidAlert};
