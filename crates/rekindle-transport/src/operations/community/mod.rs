//! Community lifecycle operations — leave.
//!
//! Orchestration logic that composes:
//! - `broadcast::dht_writes` for raw DHT primitives (create_dflt, set, open, close, watch)
//! - `broadcast::route` for route allocation
//! - `dht/*` typed modules for business logic reads/writes (governance, registry, mailbox)
//!
//! **Join is not here.** The three-phase coordinator join —
//! `submit_join_request` wrote to an inbox, `await_join_approval` waited
//! for an operator to assign a slot, `complete_join` read the registry
//! MEK vault — was replaced by the self-sovereign
//! `rekindle_governance_runtime::join_flow::run_join_stages`, which both
//! shells now drive. It had no callers left. `inbox.rs`, the operator's
//! side of that handshake, went with it.
//!
//! **Create is not here either**, for the same reason. The v1.0 flow
//! built a creator-owned registry, published the genesis MEK into a
//! registry MEK vault, and wrote the creator into a shared member index.
//! `o_cnt: 0` gives nobody a writer for the latter two, and
//! `communities-channels.md` says the MEK is *"**never** written to
//! DHT"*. Both shells now call
//! `rekindle_governance_runtime::origin::create_community`.

mod leave;

pub use leave::leave_community;

/// What a leave leaves behind for the caller to broadcast.
///
/// The departure notice is PATH 2 — peers also learn from the signed
/// `departed` row this writes into our registry slot (PATH 1), but that
/// is only observed on their next presence poll, and a departure is what
/// triggers the MEK rotation that gives forward secrecy. Telling them
/// now is the difference between seconds and a poll interval.
///
/// This used to be `leave_payload_bytes: Vec<u8>` — a postcard
/// `GossipPayload::Control(MemberLeave)`, which no desktop peer could
/// parse, and which the daemon discarded unsent anyway (`Ok(_)` in
/// `dispatch::community::lifecycle`). Returning the envelope instead of
/// bytes means the caller broadcasts it through the same signed Cap'n
/// Proto path as everything else.
pub struct LeaveResult {
    pub departure_notice: rekindle_protocol::dht::community::envelope::CommunityEnvelope,
}
