//! Phase 21 — presence primitives.
//!
//! Pure decision + timing logic for presence ops PLUS the full
//! friend-presence + community-presence orchestrators. Same chiral
//! split pattern as Phase 17 / 18 / 19 / 20: pure protocol logic
//! parameterised over Deps traits; AppState orchestration stays in
//! the src-tauri adapter.

#![forbid(unsafe_code)]

pub mod community;
pub mod deps;
pub mod friend;
pub mod friend_sync;
pub mod heartbeat;
pub mod idle;
pub mod poll_buckets;
pub mod status;

pub use community::{
    aggregate_event_rsvps, compute_merged_roles, compute_profile_diff, compute_rebuild_plan,
    gossip_degree, parse_and_classify_row, persist_discovered_registry_members,
    presence_event_id_bytes, presence_poll_tick, presence_poll_tick_public, random_peer_sample,
    role_ids_from_governance, run_initial_sync, start_presence_poll, steady_poll_duration,
    write_our_presence, ClassifiedRow, DiscoveredRow, EventRsvpEntry, GossipOverlayPlan,
    GossipOverlaySnapshot, GossipRebuildOutcome, MemberProfileSnapshot, ProfileDiffOutcome,
    MAX_SYNC_ATTEMPTS, RAPID_TICKS, RAPID_TICK_INTERVAL_SECS, STALE_HEARTBEAT_SECS,
    STALE_SYNC_RETRY_SECS, STEADY_TICK_INTERVAL_SECS, SUBKEYS_PER_SEGMENT,
};
pub use deps::{
    CommunityPresenceDeps, DiscoveredMemberRow, FriendPresenceDeps, FriendPresenceEvent,
    GameInfoSnapshot, OnlineMemberSnapshot, PresenceCredentials, PresenceError, SegmentDescriptor,
    SelfPresenceSnapshot, SetFriendStatusOutcome,
};
pub use friend::{
    handle_value_change, parse_status, parse_status_timestamp, publish_status, status_to_wire_byte,
    watch_friend, FRIEND_WATCH_SUBKEYS, PROFILE_STATUS_SUBKEY, STALE_PRESENCE_THRESHOLD_MS,
};
pub use friend_sync::{check_stale_friend_presences, sync_friends};
pub use heartbeat::{start_heartbeat_loop, HEARTBEAT_INTERVAL_SECS};
pub use idle::{decide_status_after_idle, IDLE_THRESHOLD_MS};
pub use poll_buckets::{is_member_stale, presence_poll_interval_ms, STALE_MEMBER_TTL_MS};
pub use status::{UserStatusKind, INVISIBLE_WIRE_VALUE};
