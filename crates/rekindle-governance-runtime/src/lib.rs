//! Phase 18 — community lifecycle orchestration.
//!
//! `rekindle-governance` is a pure Tier-6 CRDT merge engine ("NO I/O.
//! NO async. NO side effects."). This sibling crate layers the
//! async lifecycle ops on top — origin (community creation),
//! bootstrap (fetch origin + governance + members on join), join
//! (single-shot entry + per-stage state machine), segments (Plate
//! Gate segmentation for >255 members), and apply (the bridge that
//! wires `rekindle_governance::merge::apply()` to the in-memory
//! snapshot + UI event emit).
//!
//! The trait surface in `deps.rs` keeps all Tauri/Veilid/SQLite/Stronghold
//! types behind the adapter at `src-tauri/src/services/governance_adapter.rs`.
//! The crate itself never imports `veilid-core`, `tauri`, `rusqlite` or
//! `iota_stronghold`.
//!
//! Modules land in subsequent Phase 18 tasks (#152-#156):
//! - `apply` (task #152) — write_entry pipeline
//! - `origin` (task #153) — community creation
//! - `bootstrap` (task #154) — build bootstrap response
//! - `segments` (task #155) — Plate Gate
//! - `join` + `join_stages` (task #156) — joiner flow

#![forbid(unsafe_code)]

pub mod apply;
pub mod bootstrap;
pub mod deps;
pub mod dht_hydration;
pub mod error;
pub mod event;
pub mod join;
pub mod join_stages;
pub mod origin;
pub mod roles;
pub mod segments;

pub use apply::write_entry;
pub use bootstrap::build_bootstrap_response;
pub use deps::{
    ChannelMekSnapshot, CommunityDhtOpenSetup, CommunityInsert, CommunityMembership, DhtRecordInfo,
    DiscoveredMember, GovernanceRuntimeDeps, MekSnapshot, MemberIndexRow, OnlineMemberSnapshot,
    RecentMessageRow, UserStatusKind,
};
pub use dht_hydration::{
    hydrate_community_state_from_dht, open_community_dht_records, rebuild_governance_from_dht,
};
pub use error::GovernanceRuntimeError;
pub use event::GovernanceRuntimeEvent;
pub use join::{
    default_community_name, derive_join_identity, find_invite_in_entries, merge_presence_entry,
    InitialPresence, JoinIdentity, JoinOnlineMember,
};
pub use join_stages::{
    claim_registry_slot, collect_initial_presence_state, load_governance_snapshot, ClaimedSlot,
    GovernanceSnapshot,
};
pub use origin::create_community;
pub use segments::{
    channel_record_keys_per_segment, ensure_channel_segment_record, expand_community_segment,
    highest_segment_full, open_new_segments, segment_descriptors, SegmentDescriptor, MAX_SEGMENTS,
};
