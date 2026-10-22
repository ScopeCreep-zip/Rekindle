//! Netsim scenarios (plan C9.3, C9.4): real Hosts on a seeded simulation of
//! the network. Each runs on a single-threaded, paused runtime.

mod dm_hint_loss;
mod first_contact_retry;
mod first_login;
mod friend_watch;
mod friends;
mod group_dm;
mod harness;
mod internet_flap;
mod logs;
mod offline_day;
mod outbox_handshake;
mod pair;
mod prekey_collision;
mod prekeys;
mod rotation;
mod route_link_failure;
mod scenarios;
mod smoke;
