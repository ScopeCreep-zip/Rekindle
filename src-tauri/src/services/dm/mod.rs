//! Group DMs over SMPL records (architecture §27.2), until E2.6.
//!
//! Backed by SMPL records with `o_cnt: 0`, 3-8 member subkeys, and a MEK
//! wrapped per recipient. The `rekindle-dm` crate holds the logic; this
//! module is the desktop's entry points: accept, the store reads, and
//! frontend events. Invite and leave receive went with E2.6 G1: their
//! bodies are the sender-key ones, which `rekindle-dm` applies from G2. The 1:1 SMPL path was deleted by plan
//! E2.3 (1:1 DMs are the friend's ratchet stream).

pub mod accept;
pub mod messages;
pub mod store;
pub mod video;

pub use accept::accept_dm_invite;
pub use messages::{handle_dm_subkey_change, send_dm_message};
pub use store::{decline_dm_invite, list_dm_conversations, load_dm_messages};
