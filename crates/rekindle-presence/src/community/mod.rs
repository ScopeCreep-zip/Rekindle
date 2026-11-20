//! Phase 21 REDO — community presence orchestrators.
//!
//! Decomposed per Invariant 1 (≤500 LoC per file). Each submodule
//! owns one orchestrator + its tests:
//!
//! - `beat` — the beat engine (plan E3.4 S2): our `M`/1 beat on its
//!   randomised schedule and on explicit actions.
//! - `routes` — the member route cache (plan E3.4 S4): each member's
//!   verified `M`/6 route, replaced by a newer seq or a `RouteUnusable`.
//! - `sync` — initial-sync handshake: presence broadcast →
//!   SMPL channel-log catch-up.
//! - `registry` — registry write helpers: `write_our_presence`
//!   (build + W11.2 encrypt + W26 sign + DHT write) and
//!   `persist_discovered_registry_members` (diff + emit + batch
//!   SQLite upsert).
//! - `time` — internal `now_secs()` helper hoisted so the larger
//!   modules stay focused on orchestration.

pub mod beat;
pub mod beat_view;
pub mod on_screen;
pub mod overlay_rebuild;
pub mod policy;
pub mod poll;
pub mod profile_diff;
pub mod registry;
pub mod routes;
pub mod scan_row;
pub mod spawn;
pub mod sync;
#[cfg(test)]
pub(crate) mod test_fixture;
pub mod util;

pub use beat::{beat_delay, scheduled_state, BeatEngine, BeatSink, BEAT_DELAY_MAX, BEAT_DELAY_MIN};
pub use beat_view::{beat_is_live, live_until, BeatView, Observed, BEAT_LIVENESS_MS};
pub use on_screen::{OnScreen, MEMBERS_ON_SCREEN_MAX, ON_SCREEN_INSPECT_MS};
pub use overlay_rebuild::{
    compute_rebuild_plan, GossipOverlayPlan, GossipOverlaySnapshot, GossipRebuildOutcome,
};
pub use policy::{apply_sharing_policy, coarsen_to_bucket, filter_incoming};
pub use poll::{
    gossip_degree, presence_poll_tick, presence_poll_tick_public, presence_refresh,
    steady_poll_duration, STALE_HEARTBEAT_SECS,
};
pub use profile_diff::{compute_profile_diff, MemberProfileSnapshot, ProfileDiffOutcome};
pub use registry::{
    discovered_member_row, persist_discovered_registry_members, write_our_presence, DiscoveredRow,
    PresenceWrite,
};
pub use routes::MemberRouteCache;
pub use scan_row::{parse_and_classify_row, AcceptedRow, ClassifiedRow, SUBKEYS_PER_SEGMENT};
pub use spawn::{
    start_presence_poll, RAPID_TICKS, RAPID_TICK_INTERVAL_SECS, STEADY_TICK_INTERVAL_SECS,
};
pub use sync::run_initial_sync;
pub use util::random_peer_sample;
