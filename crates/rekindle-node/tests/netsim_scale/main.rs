//! Netsim scale scenarios (plan E3.2 S7 Verify, N = 40): the regular
//! scenarios' modules plus the long-running ones, in their own target so
//! `--test netsim` stays fast. Run with
//! `cargo test -p rekindle-node --features netsim-scale --test netsim_scale watch_ring`.

#[path = "../netsim/attachment_round_trip.rs"]
mod attachment_round_trip;
#[path = "../netsim/audit.rs"]
mod audit;
#[path = "../netsim/auto_away.rs"]
mod auto_away;
#[path = "../netsim/backfill_gate.rs"]
mod backfill_gate;
#[path = "../netsim/channel_delete_container_not_empty.rs"]
mod channel_delete_container_not_empty;
#[path = "../netsim/channel_post_both_ways.rs"]
mod channel_post_both_ways;
#[path = "../netsim/community_block_drops.rs"]
mod community_block_drops;
#[path = "../netsim/community_policy.rs"]
mod community_policy;
#[path = "../netsim/data_root.rs"]
mod data_root;
#[path = "../netsim/dm_hint_loss.rs"]
mod dm_hint_loss;
#[path = "../netsim/equivocation.rs"]
mod equivocation;
#[path = "../netsim/events.rs"]
mod events;
#[path = "../netsim/expressions.rs"]
mod expressions;
#[path = "../netsim/first_contact_receipt.rs"]
mod first_contact_receipt;
#[path = "../netsim/first_contact_retry.rs"]
mod first_contact_retry;
#[path = "../netsim/first_login.rs"]
mod first_login;
#[path = "../netsim/forward_cross_audience.rs"]
mod forward_cross_audience;
#[path = "../netsim/friend_watch.rs"]
mod friend_watch;
#[path = "../netsim/friends.rs"]
mod friends;
#[path = "../netsim/game_servers.rs"]
mod game_servers;
#[path = "../netsim/gate.rs"]
mod gate;
#[path = "../netsim/governance_warm_restore.rs"]
mod governance_warm_restore;
#[path = "../netsim/group_dm.rs"]
mod group_dm;
#[path = "../netsim/harness.rs"]
mod harness;
#[path = "../netsim/internet_flap.rs"]
mod internet_flap;
#[path = "../netsim/invite_expiry.rs"]
mod invite_expiry;
#[path = "../netsim/invite_join_leave.rs"]
mod invite_join_leave;
#[path = "../netsim/logs.rs"]
mod logs;
#[path = "../netsim/mod_report.rs"]
mod mod_report;
#[path = "../netsim/moderation.rs"]
mod moderation;
#[path = "../netsim/no_timed_inspect.rs"]
mod no_timed_inspect;
#[path = "../netsim/offline_day.rs"]
mod offline_day;
#[path = "../netsim/onboarding.rs"]
mod onboarding;
#[path = "../netsim/one_community_rebuild.rs"]
mod one_community_rebuild;
#[path = "../netsim/outbox_handshake.rs"]
mod outbox_handshake;
#[path = "../netsim/pair.rs"]
mod pair;
#[path = "../netsim/pins.rs"]
mod pins;
#[path = "../netsim/prekey_collision.rs"]
mod prekey_collision;
#[path = "../netsim/prekeys.rs"]
mod prekeys;
#[path = "../netsim/read_marker_only_advances.rs"]
mod read_marker_only_advances;
#[path = "../netsim/resume_after_restart.rs"]
mod resume_after_restart;
#[path = "../netsim/rich_messaging.rs"]
mod rich_messaging;
#[path = "../netsim/role_assign.rs"]
mod role_assign;
#[path = "../netsim/rotation.rs"]
mod rotation;
#[path = "../netsim/route_link_failure.rs"]
mod route_link_failure;
#[path = "../netsim/scenarios.rs"]
mod scenarios;
#[path = "../netsim/smoke.rs"]
mod smoke;
#[path = "../netsim/watch_ring.rs"]
mod watch_ring;
#[path = "../netsim/watch_ring_budget.rs"]
mod watch_ring_budget;
