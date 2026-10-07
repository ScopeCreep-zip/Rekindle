//! Receive-path friend authority: "is this envelope from a known active
//! friend?"
//!
//! The trait is Tier 1 so every host shares one shape. The SQLite
//! implementation lives in `rekindle-db` (`SqliteFriendStore`). Today the
//! desktop's receive path consults it through
//! `state_helpers::is_active_friend_authoritative`; the daemon wires the
//! same store when it gains `rekindle-db` (plan step D1).
//!
//! - dyn-safe via `#[async_trait]` for `Arc<dyn FriendStore>`;
//! - hex pubkeys and record keys at the boundary;
//! - [`MemoryFriendStore`] for tests.
//!
//! # Read-only by design
//!
//! Every method reads. Friend mutations (add, accept, block, remove) go
//! through the host's command surface into SQLite; this trait gives the
//! receive path one authoritative source with no in-memory cache that can
//! race with SQLite.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod memory;

pub use memory::MemoryFriendStore;

/// A friend store backend failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FriendStoreError {
    /// The storage backend reported an error.
    #[error("friend store backend: {0}")]
    Backend(String),
}

/// Friendship state, as the `friends.friendship_state` column stores it.
///
/// Authorization rule applied at receive-dispatch:
/// - `Active` — sender is authorized for any envelope variant.
/// - `PendingOut` — *we* sent a friend request and are awaiting their
///   accept. Sender is authorized only for `FriendAccept` (W16.10e
///   idempotent receive); other variants drop with a `SystemAlert`.
/// - `Removing` — friendship is being torn down. Envelopes silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FriendStatus {
    Active,
    PendingOut,
    Removing,
}

impl FriendStatus {
    /// Parse from the SQLite `friendship_state` column. The `"removing"`
    /// row value and any unknown value both map to `Removing` — the safest
    /// non-active default, so a corrupted or future-schema status row
    /// cannot auto-authorize.
    pub fn from_wire(s: &str) -> Self {
        match s {
            "accepted" => Self::Active,
            "pending_out" => Self::PendingOut,
            _ => Self::Removing,
        }
    }

    /// Wire form. Matches existing `friends.friendship_state` values.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Active => "accepted",
            Self::PendingOut => "pending_out",
            Self::Removing => "removing",
        }
    }
}

/// A friend record as seen by the receive-path. Read-only view.
///
/// All hex fields are lowercase, unprefixed, raw bytes (not base64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendRecord {
    /// Friend's Ed25519 identity pubkey (hex, 64 chars).
    /// Outer envelope signature is verified against this.
    pub pubkey_hex: String,
    /// Per-pair inbox DHT RecordKey (Phase 2 inbox-pivot, Track B).
    /// Empty during friend-add bootstrap before the per-pair inbox is created.
    pub inbox_record_key: String,
    /// Friend's mailbox DHT RecordKey for fetching their published route blob.
    /// Empty until peer publishes a mailbox.
    pub mailbox_record_key: String,
    /// Friend's most-recently-seen DeviceId (Ed25519 device pubkey, hex).
    /// `None` until they advertise via the `DeviceList` owner subkey on
    /// their per-pair inbox.
    pub current_device_id: Option<String>,
    /// Local nickname / display name. Local-only, non-spoofable.
    pub display_name: String,
    /// Microseconds since epoch when this friendship was added.
    pub added_at_us: u64,
    pub status: FriendStatus,
}

/// Receive-path read API for the friend authority.
///
/// Implementations: [`MemoryFriendStore`] (in-process, tests) and
/// `rekindle_db::SqliteFriendStore` (the hosts).
#[async_trait]
pub trait FriendStore: Send + Sync {
    /// Lookup by identity pubkey. Used by `dispatch_inbound` on every
    /// inbound envelope to authorize the sender. Hot path: must be fast.
    async fn lookup_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<Option<FriendRecord>, FriendStoreError>;

    /// Lookup by per-pair inbox RecordKey. Used when a DHT watch fires
    /// on an inbox and we need to route the change to its peer.
    async fn lookup_by_inbox_record_key(
        &self,
        owner_key: &str,
        inbox_record_key: &str,
    ) -> Result<Option<FriendRecord>, FriendStoreError>;

    /// Batch lookup by identity pubkey. Used when a single
    /// `VeilidValueChange` touches multiple subkeys from multiple peers
    /// and we want one `WHERE public_key IN (?, ?, ...)` query instead of N.
    async fn lookup_batch_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hexes: &[String],
    ) -> Result<Vec<FriendRecord>, FriendStoreError>;

    /// All `Active` friends for this owner. Used at attach time to install
    /// per-pair inbox watches for each.
    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, FriendStoreError>;

    /// Boolean test for receive-path gating. Replaces
    /// `src-tauri/src/state_helpers::is_friend`.
    /// Returns `true` only for `FriendStatus::Active`.
    async fn is_active_friend(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<bool, FriendStoreError> {
        Ok(self
            .lookup_by_pubkey(owner_key, pubkey_hex)
            .await?
            .is_some_and(|f| f.status == FriendStatus::Active))
    }
}
