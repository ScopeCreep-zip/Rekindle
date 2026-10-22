//! Netsim scenarios (plan C9.3, C9.4): real Hosts on a seeded simulation of
//! the network. Each runs on a single-threaded, paused runtime.

mod auto_away;
mod dm_hint_loss;
mod first_contact_retry;
mod first_login;
mod friend_watch;
mod friends;
mod governance_warm_restore;
mod group_dm;
mod harness;
mod internet_flap;
mod invite_expiry;
mod logs;
mod no_timed_inspect;
mod offline_day;
mod one_community_rebuild;
mod outbox_handshake;
mod pair;
mod prekey_collision;
mod prekeys;
mod resume_after_restart;
mod rotation;
mod route_link_failure;
mod scenarios;
mod smoke;
