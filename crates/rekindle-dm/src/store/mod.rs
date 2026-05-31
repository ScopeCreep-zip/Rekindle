//! Phase 13 — DM persistence port + types.
//!
//! Owns the `DmStore` trait (the port the rekindle-dm domain logic
//! talks to) plus the shared input/output types. The concrete
//! `SqliteDmStore` impl lives next door in `sqlite.rs`; the split
//! keeps each file's behavior LoC under the Invariant-1 cap.
//!
//! Pre-Phase-13 this lived in `src-tauri/services/dm/store.rs` and
//! reached into `Arc<AppState>` via `state_helpers` for owner_key
//! extraction — the trait surface takes `owner_key` explicitly so the
//! crate stays free of `AppState`.
//!
//! Plan reference: § Phase 13 of
//! `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::DmError;
use crate::invite::GroupDmParticipant;

mod sqlite;
pub use sqlite::SqliteDmStore;

/// One DM conversation row, as projected for the UI conversation list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmConversation {
    pub record_key: String,
    pub is_group: bool,
    pub initiator_public_key: String,
    pub initiator_pseudonym: String,
    pub my_subkey: u32,
    pub participants: Vec<GroupDmParticipant>,
    pub mek_generation: u32,
    pub created_at: i64,
    pub last_message_at: Option<i64>,
}

/// One persisted DM message row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmMessageRecord {
    pub id: i64,
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp: i64,
    pub sequence: i64,
    pub mek_generation: i64,
}

/// Bundle of fields needed to persist a pending DM invite (1:1 or group).
/// Avoids the 10-arg signature of the previous free function while
/// keeping each field explicit at the call site.
#[derive(Debug, Clone)]
pub struct DmInvitePending {
    pub record_key: String,
    pub is_group: bool,
    pub initiator_public_key: String,
    pub initiator_pseudonym: String,
    pub my_subkey: u32,
    pub participants: Vec<GroupDmParticipant>,
    pub mek_generation: u32,
    pub slot_seed_hex: String,
    pub wrapped_mek_blob: Option<Vec<u8>>,
    pub created_at: i64,
}

/// Session metadata returned by [`DmStore::get_session_meta`]. Holds the
/// per-conversation fields the sender/receiver needs every time they
/// process a message — `my_subkey` for routing, `initiator_pseudonym`
/// for display, `is_group` for code-path selection, `slot_seed` for
/// per-subkey Ed25519 derivation.
#[derive(Debug, Clone)]
pub struct DmSessionMeta {
    pub my_subkey: u32,
    pub initiator_pseudonym: String,
    pub initiator_public_key: String,
    pub is_group: bool,
    /// 32-byte slot seed, decoded from `slot_seed_hex`.
    pub slot_seed: [u8; 32],
}

/// One message to insert into `dm_messages`. The `last_message_at`
/// column on the parent `dms` row is updated as part of the same call.
#[derive(Debug, Clone)]
pub struct DmMessageInsert {
    pub record_key: String,
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp_secs: i64,
    pub sequence: u64,
    pub mek_generation: u64,
}

/// Full invite metadata needed by the responder (`accept_dm_invite`)
/// to restore the MEK chain and open the DHT record.
#[derive(Debug, Clone)]
pub struct DmInviteMeta {
    pub initiator_public_key: String,
    pub my_subkey: u32,
    pub mek_generation: u64,
    pub is_group: bool,
    pub wrapped_mek_blob: Option<Vec<u8>>,
    pub participants: Vec<GroupDmParticipant>,
}

/// Persistence port for the DM domain.
///
/// Production impl is `SqliteDmStore` (in `sqlite.rs`). Tests can
/// supply in-memory mocks. Every method takes `owner_key` explicitly
/// — the crate has no `AppState` access, so multi-user scoping is the
/// caller's responsibility (set by the src-tauri adapter via
/// `state_helpers::owner_key_or_default`).
#[async_trait]
pub trait DmStore: Send + Sync {
    async fn persist_invite_pending(
        &self,
        owner_key: &str,
        invite: DmInvitePending,
    ) -> Result<(), DmError>;

    async fn list_conversations(
        &self,
        owner_key: &str,
    ) -> Result<Vec<DmConversation>, DmError>;

    async fn load_messages(
        &self,
        owner_key: &str,
        record_key: &str,
        limit: i64,
    ) -> Result<Vec<DmMessageRecord>, DmError>;

    async fn decline_invite(
        &self,
        owner_key: &str,
        record_key: &str,
    ) -> Result<(), DmError>;

    /// Read per-session metadata (my_subkey + pseudonym + slot_seed +
    /// is_group). Returns `None` if the conversation row is missing.
    async fn get_session_meta(
        &self,
        owner_key: &str,
        record_key: &str,
    ) -> Result<Option<DmSessionMeta>, DmError>;

    /// Next outbound sequence for `sender_pseudonym` in `record_key`.
    /// Returns 1 if no prior messages exist. (Sequence is per-sender —
    /// each peer increments their own monotonic counter.)
    async fn next_sequence_for_sender(
        &self,
        owner_key: &str,
        record_key: &str,
        sender_pseudonym: &str,
    ) -> Result<u64, DmError>;

    /// Insert a single message into `dm_messages` and bump the parent
    /// `dms.last_message_at`. Used by both outbound (own send) and
    /// inbound (received + decrypted) paths.
    async fn persist_message(
        &self,
        owner_key: &str,
        msg: DmMessageInsert,
    ) -> Result<(), DmError>;

    /// Oldest message timestamp (seconds) within the last `lookback`
    /// sequences for a conversation. Used by the ratchet trigger to
    /// detect whether 24h has elapsed since the start of the current
    /// 100-message window.
    async fn oldest_recent_message_ts(
        &self,
        owner_key: &str,
        record_key: &str,
        lookback: i64,
    ) -> Result<Option<i64>, DmError>;

    /// Update the conversation's persisted MEK generation after a
    /// ratchet advance.
    async fn update_mek_generation(
        &self,
        owner_key: &str,
        record_key: &str,
        new_generation: u32,
    ) -> Result<(), DmError>;

    /// Read full invite metadata for the responder (`accept_dm_invite`).
    /// Returns `None` if no row exists. Includes the wrapped MEK blob
    /// (group DMs) and full participant list (for watch-set selection).
    async fn load_invite_meta(
        &self,
        owner_key: &str,
        record_key: &str,
    ) -> Result<Option<DmInviteMeta>, DmError>;
}
