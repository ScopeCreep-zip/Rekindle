//! Transport-agnostic gossip mesh primitives for Rekindle v2.0.
//!
//! Tier 5 utility crate: peer selection, deduplication, Lamport clocks,
//! sender-side rate limiting, and generic broadcast helpers.
//!
//! Phase 20 REDO adds the full chiral-split `mesh_broadcast` +
//! `peer_select` modules so the entire gossip pipeline (sign + dedup
//! + lamport-bump + reliability-weighted fan-out + supervised
//! per-peer retry) lives in the crate, parameterised over the
//! `GossipDeps` trait. The pre-port src-tauri module collapses to a
//! thin facade.

pub mod broadcast;
pub mod dedup;
pub mod deps;
pub mod lamport;
pub mod mesh;
pub mod mesh_broadcast;
pub mod peer_select;
pub mod rate_limit;

pub use deps::{GossipDeps, GossipError, PeerInfo};
pub use mesh_broadcast::{send_to_mesh, send_to_mesh_raw, MAX_PENDING_MESH};
pub use peer_select::{scores_from_counters, sort_peers_by_reliability};
