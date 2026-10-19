//! `rekindle-dm` — group DMs (architecture §27.2).
//!
//! A group DM is an SMPL record with `o_cnt: 0` (Schwarzschild — no
//! creator reserved subkeys), one member subkey each, and a random MEK
//! wrapped per recipient. The 1:1 SMPL path (2 member subkeys, an ECDH
//! MEK) was deleted by plan E2.3: 1:1 DMs are the friend's ratchet
//! stream. E2.6 replaces the group path with sender-owned keys.
//!
//! Phase 13 expanded this crate from pure-logic (types + MEK) into the
//! DM domain owner: adds `store` (DmStore trait + SqliteDmStore impl)
//! to consolidate persistence behind a port. DHT-touching orchestration
//! stays in `src-tauri/services/dm/` per Invariant 2 — only
//! `rekindle-transport` and `rekindle-protocol` may import `veilid-core`.

pub mod deps; // Phase 13 — DmDeps + DmMekCache + DmEvent (orchestration ports).
pub mod envelope; // Phase 13 — DmCiphertext + encrypt/decrypt helpers.
pub mod error;
pub mod ingest; // Phase 13 — handle_incoming_group_dm_invite / leave.
pub mod invite;
pub mod mek;
pub mod receiver; // Phase 13 — handle_dm_subkey_change (inbound).
pub mod sender; // Phase 13 — send_dm_message + maybe_ratchet.
pub mod session; // Phase 13 — accept_dm_invite (lifecycle).
pub mod store; // Phase 13 — DmStore trait + SqliteDmStore impl.
pub mod video; // Phase 13 — DmVideoReassemblyState (pure buffer ops).

pub use deps::{DmDeps, DmEvent, DmMekCache};
pub use envelope::{
    build_envelope, decrypt_body, parse_envelope, DmCiphertext, DM_RATCHET_MESSAGE_INTERVAL,
    DM_RATCHET_TIME_INTERVAL_SECS,
};
pub use error::DmError;
pub use ingest::{
    handle_incoming_dm_leave, handle_incoming_group_dm_invite, IncomingGroupDmInvite,
};
pub use invite::{GroupDmInvite, GroupDmParticipant};
pub use mek::{ratchet_dm_mek, DmMek, DmMekChain, MEK_LEN};
pub use receiver::handle_dm_subkey_change;
pub use sender::send_dm_message;
pub use session::accept_dm_invite;
pub use store::{
    DmConversation, DmInviteMeta, DmInvitePending, DmMessageInsert, DmMessageRecord, DmSessionMeta,
    DmStore, SqliteDmStore,
};
pub use video::{AssembledFrame, DmVideoFragment, DmVideoReassemblyState, FRAGMENT_PAYLOAD_LIMIT};
