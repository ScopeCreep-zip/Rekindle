//! Community lifecycle operations — create, join, leave.
//!
//! Orchestration logic that composes:
//! - `broadcast::dht_writes` for raw DHT primitives (create_dflt, set, open, close, watch)
//! - `broadcast::route` for route allocation
//! - `dht/*` typed modules for business logic reads/writes (governance, registry, mailbox)
//!
//! The join flow is split into three phases for event-driven completion:
//! - `submit_join_request` — writes to inbox, returns immediately
//! - `await_join_approval` — select! between registry poll (tier 3) and direct notification (tier 2)
//! - `complete_join` — reads channels + MEKs after approval confirmed
//!
//! `join_community` is a convenience wrapper that calls all three sequentially.

use crate::payload::rpc::ChannelEntrySummary;

mod create;
mod inbox;
mod join;
mod leave;

pub use create::create_community;
pub use inbox::read_inbox_requests;
pub use join::{await_join_approval, complete_join, join_community, submit_join_request};
pub use leave::leave_community;

pub struct CommunityCreated {
    pub governance_key: String,
    pub governance_keypair_bytes: Vec<u8>,
    pub registry_key: String,
    pub registry_keypair_bytes: Vec<u8>,
    pub community_mailbox_key: String,
    pub join_inbox_key: String,
    pub default_channel_id: String,
    pub our_pseudonym_key: String,
    pub our_slot_index: u32,
    pub mek_generation: u64,
}

/// Returned by `submit_join_request` — metadata needed for approval await + completion.
pub struct JoinRequestSubmitted {
    pub community_name: String,
    pub governance_key: String,
    pub our_pseudonym_hex: String,
    pub registry_key: String,
    pub community_mailbox_key: String,
}

pub struct JoinResult {
    pub community_name: String,
    pub governance_key: String,
    pub our_pseudonym_key: String,
    pub display_name: String,
    pub our_slot_index: u32,
    pub registry_key: String,
    pub community_mailbox_key: String,
    pub channels: Vec<ChannelEntrySummary>,
    pub meks_cached: usize,
    pub slot_seed: [u8; 32],
}

pub struct LeaveResult {
    pub leave_payload_bytes: Vec<u8>,
}

// ── Utilities ───────────────────────────────────────────────────────────

/// Shared by the join and leave flows to pick a deterministic inbox
/// subkey for a given pseudonym.
fn pseudonym_to_inbox_subkey(pseudonym_hex: &str) -> u32 {
    let hash = blake3::hash(pseudonym_hex.as_bytes());
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        % crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT
}
