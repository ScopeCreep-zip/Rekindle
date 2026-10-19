//! Group DMs over SMPL records (architecture §27.2), until E2.6.
//!
//! Backed by SMPL records with `o_cnt: 0`, 3-8 member subkeys, and a MEK
//! wrapped per recipient. The `rekindle-dm` crate holds the logic; this
//! module is the desktop's entry points: invite receive, accept, the
//! store reads, and frontend events. The 1:1 SMPL path was deleted by plan
//! E2.3 (1:1 DMs are the friend's ratchet stream).

pub mod accept;
pub mod ingest;
pub mod messages;
pub mod store;
pub mod video;

pub use accept::accept_dm_invite;
pub use ingest::{handle_incoming_dm_leave, handle_incoming_group_dm_invite};
pub use messages::{handle_dm_subkey_change, send_dm_message};
pub use store::{decline_dm_invite, list_dm_conversations, load_dm_messages};
