//! DM persistence port — trait + vocabulary types.
//!
//! Lives in rekindle-types so both rekindle-storage (impl) and
//! rekindle-chat (consumer) can import it without circular deps.
//! Follows the same pattern as [`Transport`](crate::transport) trait.
//!
//! All methods are synchronous (`&self`). VaultStore is synchronous
//! (`Mutex<Connection>`). The node crate wraps in `spawn_blocking`
//! where needed.
//!
//! No `owner_key` parameter — single identity per vault in new gen.

use serde::{Deserialize, Serialize};

use crate::EpochMs;

// ── Participant descriptor ──────────────────────────────────────────

/// One participant slot in a DM conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DmParticipant {
    /// Display name. Empty if not yet known (responder slot before acceptance).
    pub pseudonym: String,
    /// SMPL subkey this participant writes to.
    pub subkey: u32,
    /// Hex Ed25519 identity public key. Empty if not yet known.
    pub public_key: String,
}

// ── Input types (caller → store) ────────────────────────────────────

/// Fields to persist a pending DM invite (1:1 or group).
/// Idempotent on `record_key` (`ON CONFLICT DO NOTHING`).
#[derive(Debug, Clone)]
pub struct DmInvitePending {
    pub record_key: String,
    pub is_group: bool,
    pub initiator_public_key: String,
    pub initiator_pseudonym: String,
    pub my_subkey: u32,
    pub participants: Vec<DmParticipant>,
    pub mek_generation: u32,
    pub slot_seed_hex: String,
    pub wrapped_mek_blob: Option<Vec<u8>>,
}

/// One message to insert. Used by both outbound (own send) and
/// inbound (received + decrypted) paths.
#[derive(Debug, Clone)]
pub struct DmMessageInsert {
    pub record_key: String,
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp_ms: EpochMs,
    pub sequence: u64,
    pub mek_generation: u64,
    pub is_self: bool,
}

// ── Output types (store → caller) ───────────────────────────────────

/// Per-conversation metadata needed on every send/receive.
#[derive(Debug, Clone)]
pub struct DmSessionMeta {
    pub my_subkey: u32,
    pub initiator_pseudonym: String,
    pub initiator_public_key: String,
    /// The OTHER party's Ed25519 public key hex.
    /// For 1:1 DMs: whichever participant has a different subkey than
    /// `my_subkey`. Computed from the participants list at query time.
    /// Used as the key for Triple Ratchet session lookup.
    pub peer_public_key: String,
    pub is_group: bool,
    /// 32-byte slot seed decoded from `slot_seed_hex`.
    pub slot_seed: [u8; 32],
}

/// Full invite metadata for the responder (`accept_dm_invite`).
#[derive(Debug, Clone)]
pub struct DmInviteMeta {
    pub initiator_public_key: String,
    pub my_subkey: u32,
    pub mek_generation: u64,
    pub is_group: bool,
    pub wrapped_mek_blob: Option<Vec<u8>>,
    pub participants: Vec<DmParticipant>,
}

/// One DM conversation row for UI listing.
#[derive(Debug, Clone, Serialize)]
pub struct DmConversation {
    pub record_key: String,
    pub is_group: bool,
    pub initiator_public_key: String,
    pub initiator_pseudonym: String,
    pub my_subkey: u32,
    pub participants: Vec<DmParticipant>,
    pub mek_generation: u32,
    pub last_message_at: Option<EpochMs>,
}

/// One persisted DM message row.
#[derive(Debug, Clone, Serialize)]
pub struct DmMessageRecord {
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp: EpochMs,
    pub sequence: u64,
    pub mek_generation: u64,
    pub is_self: bool,
}

// ── Error ───────────────────────────────────────────────────────────

/// Errors from DmStore operations. Kept minimal — the store is a port,
/// not a policy layer.
#[derive(Debug, thiserror::Error)]
pub enum DmStoreError {
    #[error("dm store: {0}")]
    Storage(String),
    #[error("dm store: not found: {0}")]
    NotFound(String),
    #[error("dm store: invalid data: {0}")]
    InvalidData(String),
}

// ── Trait ────────────────────────────────────────────────────────────

/// Persistence port for the DM domain.
///
/// Production: `impl DmStore for VaultStore` in `rekindle-storage/src/dm/mod.rs`.
/// Tests: in-memory mock.
///
/// All methods are `&self` (not `&mut self`) — VaultStore uses
/// `Mutex<Connection>` internally. All methods are synchronous.
pub trait DmStore: Send + Sync {
    /// Persist a pending DM invite. Idempotent on `record_key`.
    fn dm_persist_invite(
        &self, invite: DmInvitePending,
    ) -> Result<(), DmStoreError>;

    /// List all DM conversations, most recent first.
    fn dm_list_conversations(&self) -> Result<Vec<DmConversation>, DmStoreError>;

    /// Load messages for a conversation, oldest first, capped at `limit`.
    fn dm_load_messages(
        &self, record_key: &str, limit: u32,
    ) -> Result<Vec<DmMessageRecord>, DmStoreError>;

    /// Delete a conversation and its messages (decline invite or leave).
    fn dm_decline_invite(
        &self, record_key: &str,
    ) -> Result<(), DmStoreError>;

    /// Read per-session metadata. `None` if row missing.
    fn dm_get_session_meta(
        &self, record_key: &str,
    ) -> Result<Option<DmSessionMeta>, DmStoreError>;

    /// Next outbound sequence for a sender. Returns 1 if none exist.
    fn dm_next_sequence(
        &self, record_key: &str, sender_pseudonym: &str,
    ) -> Result<u64, DmStoreError>;

    /// Insert a message and bump `last_message_at` on the conversation.
    fn dm_persist_message(
        &self, msg: DmMessageInsert,
    ) -> Result<(), DmStoreError>;

    /// Oldest message timestamp (secs) within last `lookback` sequences.
    /// Used by ratchet trigger to detect 24h elapsed.
    fn dm_oldest_recent_ts(
        &self, record_key: &str, lookback: i64,
    ) -> Result<Option<i64>, DmStoreError>;

    /// Update persisted MEK generation after ratchet advance.
    fn dm_update_mek_generation(
        &self, record_key: &str, new_gen: u32,
    ) -> Result<(), DmStoreError>;

    /// Check if a message hash is known (replay detection).
    fn is_dm_hash_known(&self, hash: &[u8; 32]) -> Result<bool, DmStoreError>;

    /// Store a message hash after successful decrypt.
    fn store_dm_hash(
        &self, hash: &[u8; 32], record_key: &str,
    ) -> Result<(), DmStoreError>;

    /// Sweep expired message hashes older than `max_age_secs`.
    fn sweep_dm_hashes(&self, max_age_secs: i64) -> Result<u64, DmStoreError>;

    /// Load full invite metadata for the responder.
    fn dm_load_invite_meta(
        &self, record_key: &str,
    ) -> Result<Option<DmInviteMeta>, DmStoreError>;
}
