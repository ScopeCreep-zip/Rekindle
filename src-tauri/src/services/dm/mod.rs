//! Direct messages (architecture §27).
//!
//! Backed by SMPL records with `o_cnt: 0`, exactly 2 (or 3-8 for group)
//! member subkeys, and a MEK derived deterministically via X25519 ECDH
//! (2-party) or wrapped per-recipient (group). The `rekindle-dm` crate
//! holds the pure-logic layer (MEK derivation, types). This module is
//! the orchestration: SMPL record creation, invite send/receive,
//! local persistence, and frontend events.

pub mod accept;
pub mod create;
pub mod ingest;
pub mod messages;
pub mod store;

pub use accept::accept_dm_invite;
pub use create::start_dm;
pub use ingest::{
    handle_incoming_dm_decline, handle_incoming_dm_invite, handle_incoming_dm_leave,
    handle_incoming_group_dm_invite,
};
pub use messages::{handle_dm_subkey_change, send_dm_message};
pub use store::{decline_dm_invite, list_dm_conversations, load_dm_messages};
