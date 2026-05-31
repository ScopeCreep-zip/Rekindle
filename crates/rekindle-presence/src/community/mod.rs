//! Phase 21 REDO — community presence orchestrators.
//!
//! Decomposed per Invariant 1 (≤500 LoC per file). Each submodule
//! owns one orchestrator + its tests:
//!
//! - `sync` — initial-sync handshake: presence broadcast →
//!   per-channel SyncRequest → SMPL channel-log catch-up.
//! - `registry` — registry write helpers: `write_our_presence`
//!   (build + W11.2 encrypt + W26 sign + DHT write) and
//!   `persist_discovered_registry_members` (diff + emit + batch
//!   SQLite upsert).
//! - `time` — internal `now_secs()` helper hoisted so the larger
//!   modules stay focused on orchestration.

pub mod overlay_rebuild;
pub mod poll;
pub mod profile_diff;
pub mod registry;
pub mod role_merge;
pub mod rsvp_aggregate;
pub mod scan_row;
pub mod spawn;
pub mod sync;
#[cfg(test)]
pub(crate) mod test_fixture;
mod time;
pub mod util;

pub use poll::{
    gossip_degree, presence_poll_tick, presence_poll_tick_public, steady_poll_duration,
    MAX_SYNC_ATTEMPTS, STALE_HEARTBEAT_SECS, STALE_SYNC_RETRY_SECS,
};
pub use scan_row::{parse_and_classify_row, AcceptedRow, ClassifiedRow, SUBKEYS_PER_SEGMENT};
pub use registry::{
    persist_discovered_registry_members, write_our_presence, DiscoveredRow,
};
pub use overlay_rebuild::{
    compute_rebuild_plan, GossipOverlayPlan, GossipOverlaySnapshot, GossipRebuildOutcome,
};
pub use profile_diff::{compute_profile_diff, MemberProfileSnapshot, ProfileDiffOutcome};
pub use role_merge::compute_merged_roles;
pub use rsvp_aggregate::{aggregate_event_rsvps, EventRsvpEntry};
pub use spawn::{
    start_presence_poll, RAPID_TICKS, RAPID_TICK_INTERVAL_SECS, STEADY_TICK_INTERVAL_SECS,
};
pub use sync::run_initial_sync;
pub use util::{presence_event_id_bytes, random_peer_sample, role_ids_from_governance};
