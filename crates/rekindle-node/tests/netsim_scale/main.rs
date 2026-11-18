//! Netsim scale scenarios (plan E3.2 S7 Verify, N = 40): the regular
//! scenarios' modules plus the long-running ones, in their own target so
//! `--test netsim` stays fast. Run with
//! `cargo test -p rekindle-node --features netsim-scale --test netsim_scale watch_ring`.

#[path = "../netsim/answered_request.rs"]
mod answered_request;
#[path = "../netsim/attachment_round_trip.rs"]
mod attachment_round_trip;
#[path = "../netsim/audit.rs"]
mod audit;
#[path = "../netsim/auto_away.rs"]
mod auto_away;
#[path = "../netsim/backfill_gate.rs"]
mod backfill_gate;
#[path = "../netsim/behind_key_repair.rs"]
mod behind_key_repair;
#[path = "../netsim/channel_delete_container_not_empty.rs"]
mod channel_delete_container_not_empty;
#[path = "../netsim/channel_post_both_ways.rs"]
mod channel_post_both_ways;
#[path = "../netsim/community_block_drops.rs"]
mod community_block_drops;
#[path = "../netsim/community_info_counts.rs"]
mod community_info_counts;
#[path = "../netsim/community_policy.rs"]
mod community_policy;
#[path = "../netsim/crossing_request.rs"]
mod crossing_request;
#[path = "../netsim/data_root.rs"]
mod data_root;
#[path = "../netsim/dm_ack_loss.rs"]
mod dm_ack_loss;
#[path = "../netsim/dm_ack_through.rs"]
mod dm_ack_through;
#[path = "../netsim/dm_after_cancel.rs"]
mod dm_after_cancel;
#[path = "../netsim/dm_append_deadline.rs"]
mod dm_append_deadline;
#[path = "../netsim/dm_before_accept.rs"]
mod dm_before_accept;
#[path = "../netsim/dm_chain_watch.rs"]
mod dm_chain_watch;
#[path = "../netsim/dm_hint_loss.rs"]
mod dm_hint_loss;
#[path = "../netsim/dm_new_friendship.rs"]
mod dm_new_friendship;
#[path = "../netsim/equivocation.rs"]
mod equivocation;
#[path = "../netsim/events.rs"]
mod events;
#[path = "../netsim/expressions.rs"]
mod expressions;
#[path = "../netsim/first_contact_reads.rs"]
mod first_contact_reads;
#[path = "../netsim/first_contact_receipt.rs"]
mod first_contact_receipt;
#[path = "../netsim/first_contact_retry.rs"]
mod first_contact_retry;
#[path = "../netsim/first_login.rs"]
mod first_login;
#[path = "../netsim/forward_cross_audience.rs"]
mod forward_cross_audience;
#[path = "../netsim/friend_list_record.rs"]
mod friend_list_record;
#[path = "../netsim/friend_name.rs"]
mod friend_name;
#[path = "../netsim/friend_watch.rs"]
mod friend_watch;
#[path = "../netsim/friends.rs"]
mod friends;
#[path = "../netsim/friends_key_receipt.rs"]
mod friends_key_receipt;
#[path = "../netsim/game_servers.rs"]
mod game_servers;
#[path = "../netsim/gate.rs"]
mod gate;
#[path = "../netsim/gossip_after_unlock.rs"]
mod gossip_after_unlock;
#[path = "../netsim/governance_partial.rs"]
mod governance_partial;
#[path = "../netsim/governance_publication.rs"]
mod governance_publication;
#[path = "../netsim/governance_queued.rs"]
mod governance_queued;
#[path = "../netsim/governance_warm_restore.rs"]
mod governance_warm_restore;
#[path = "../netsim/group_dm.rs"]
mod group_dm;
#[path = "../netsim/harness.rs"]
mod harness;
#[path = "../netsim/host_shutdown.rs"]
mod host_shutdown;
#[path = "../netsim/hung_call.rs"]
mod hung_call;
#[path = "../netsim/hung_send.rs"]
mod hung_send;
#[path = "../netsim/internet_flap.rs"]
mod internet_flap;
#[path = "../netsim/invisible_game.rs"]
mod invisible_game;
#[path = "../netsim/invite_expiry.rs"]
mod invite_expiry;
#[path = "../netsim/invite_join_leave.rs"]
mod invite_join_leave;
#[path = "../netsim/invite_unpublished.rs"]
mod invite_unpublished;
#[path = "../netsim/join_from_hint.rs"]
mod join_from_hint;
#[path = "../netsim/kick_rotates_once.rs"]
mod kick_rotates_once;
#[path = "../netsim/kicked_slot_write.rs"]
mod kicked_slot_write;
#[path = "../netsim/left_quiet.rs"]
mod left_quiet;
#[path = "../netsim/left_slot_write.rs"]
mod left_slot_write;
#[path = "../netsim/logout_unlock.rs"]
mod logout_unlock;
#[path = "../netsim/logs.rs"]
mod logs;
#[path = "../netsim/mek_transfer_retry.rs"]
mod mek_transfer_retry;
#[path = "../netsim/member_events.rs"]
mod member_events;
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
#[path = "../netsim/outbox_stream_independence.rs"]
mod outbox_stream_independence;
#[path = "../netsim/own_removal.rs"]
mod own_removal;
#[path = "../netsim/own_removal_quiet.rs"]
mod own_removal_quiet;
#[path = "../netsim/pair.rs"]
mod pair;
#[path = "../netsim/pins.rs"]
mod pins;
#[path = "../netsim/post_before_key.rs"]
mod post_before_key;
#[path = "../netsim/prekey_collision.rs"]
mod prekey_collision;
#[path = "../netsim/prekeys.rs"]
mod prekeys;
#[path = "../netsim/read_marker_only_advances.rs"]
mod read_marker_only_advances;
#[path = "../netsim/refriend_after_removal.rs"]
mod refriend_after_removal;
#[path = "../netsim/refused_request.rs"]
mod refused_request;
#[path = "../netsim/rejoin_after_leave.rs"]
mod rejoin_after_leave;
#[path = "../netsim/removal_ack.rs"]
mod removal_ack;
#[path = "../netsim/resume_after_restart.rs"]
mod resume_after_restart;
#[path = "../netsim/rich_messaging.rs"]
mod rich_messaging;
#[path = "../netsim/ring_census.rs"]
mod ring_census;
#[path = "../netsim/ring_frontier.rs"]
mod ring_frontier;
#[path = "../netsim/role_assign.rs"]
mod role_assign;
#[path = "../netsim/rotation.rs"]
mod rotation;
#[path = "../netsim/route_link_failure.rs"]
mod route_link_failure;
#[path = "../netsim/route_liveness.rs"]
mod route_liveness;
#[path = "../netsim/scenarios.rs"]
mod scenarios;
#[path = "../netsim/sending_route.rs"]
mod sending_route;
#[path = "../netsim/smoke.rs"]
mod smoke;
#[path = "../netsim/stale_route_call.rs"]
mod stale_route_call;
#[path = "../netsim/stall.rs"]
mod stall;
#[path = "../netsim/udp_blocked.rs"]
mod udp_blocked;
#[path = "../netsim/wake_trigger.rs"]
mod wake_trigger;
#[path = "../netsim/walk_cost.rs"]
mod walk_cost;
#[path = "../netsim/watch_fallback.rs"]
mod watch_fallback;
#[path = "../netsim/watch_ring.rs"]
mod watch_ring;
#[path = "../netsim/watch_ring_budget.rs"]
mod watch_ring_budget;
