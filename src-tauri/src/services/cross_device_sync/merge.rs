//! Phase 22.e-REDO — thin facade.
//!
//! Pure CRDT merge rules (architecture §28.4) live in
//! `rekindle_sync::cross_device::merge`. Pre-port these were
//! already pure functions; the relocation removes a duplicate
//! source-of-truth under Invariant 7 (every CRDT decision lives in
//! a domain crate).
//!
//! Re-exports preserve the `super::merge::*` import paths used by
//! `cross_device_sync/watch.rs`.

pub use rekindle_sync::{merge_device_list, merge_manifest, merge_preferences, merge_read_state};
