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

pub struct LeaveResult {
    pub leave_payload_bytes: Vec<u8>,
}

// ── Utilities ───────────────────────────────────────────────────────────

/// Deterministic inbox subkey for a pseudonym.
///
/// Only the leave flow still uses it, to announce a departure in the
/// join inbox. Nothing reads that inbox any more — the operator-side
/// reader went with the coordinator join — so this write is on the list
/// to be replaced by the leaver zeroing its own registry slot
/// (`communities-governance.md` §"Leave and rejoin").
fn pseudonym_to_inbox_subkey(pseudonym_hex: &str) -> u32 {
    let hash = blake3::hash(pseudonym_hex.as_bytes());
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        % crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT
}
