//! Track A.1 — Receive-path friend authority.
//!
//! Storage trait the receive-dispatch layer (`subscriptions/dispatch.rs`)
//! consults to verify "is this envelope from a known active friend." Replaces
//! the in-memory `state.friends` map in `src-tauri` that suffered a hydration
//! race: the Veilid dispatch loop spawned at app boot, before SQLite was
//! loaded, so inbound envelopes during the gap failed authorization with
//! "not friends even though we are."
//!
//! Mirrors the [`EnvelopeStore`](crate::envelope_store::EnvelopeStore) pattern:
//! - dyn-safe via `#[async_trait]` for `Arc<dyn FriendStore>`
//! - String hex pubkeys/RecordKeys at the boundary (matches existing wire
//!   convention used by `EnvelopeStore::owner_key`/`recipient_key`)
//! - [`StoreError`] reused from `envelope_store`
//! - In-memory impl for tests; SQLite impl lives in `src-tauri/src/services/`.
//!
//! # Read-only by design
//!
//! All methods are read-only. Friend mutations (add, accept, block, remove)
//! happen via the existing `src-tauri` command surface writing directly to
//! SQLite. This trait exists to give the transport's receive-dispatch layer
//! a single authoritative source — no in-memory cache that can race with
//! SQLite.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod memory;

pub use memory::MemoryFriendStore;

use crate::envelope_store::StoreError;

/// Friendship state. Mirrors the existing `FriendshipState` enum in
/// `src-tauri/src/state.rs` and the `friends.friendship_state` column.
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
/// Three host impls today:
/// - [`MemoryFriendStore`] — in-process `HashMap` (this crate, tests).
/// - `SqliteFriendStore` — provided by `src-tauri/src/services/friend_store_sqlite.rs`,
///   uses the existing rusqlite pool. Lives in `src-tauri` because rusqlite
///   is heavy and Tauri is the only host shipping it. (See Track A.2.)
/// - `JsonFriendStore` — atomic JSON file for `rekindle-cli`/`rekindle-node`
///   (added when those frontends migrate; not required for Track A).
#[async_trait]
pub trait FriendStore: Send + Sync {
    /// Lookup by identity pubkey. Used by `dispatch_inbound` on every
    /// inbound envelope to authorize the sender. Hot path: must be fast.
    async fn lookup_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<Option<FriendRecord>, StoreError>;

    /// Lookup by per-pair inbox RecordKey. Used when a DHT watch fires
    /// on an inbox and we need to route the change to its peer.
    async fn lookup_by_inbox_record_key(
        &self,
        owner_key: &str,
        inbox_record_key: &str,
    ) -> Result<Option<FriendRecord>, StoreError>;

    /// Batch lookup by identity pubkey. Used when a single
    /// `VeilidValueChange` touches multiple subkeys from multiple peers
    /// and we want one `WHERE public_key IN (?, ?, ...)` query instead of N.
    async fn lookup_batch_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hexes: &[String],
    ) -> Result<Vec<FriendRecord>, StoreError>;

    /// All `Active` friends for this owner. Used at attach time to install
    /// per-pair inbox watches for each.
    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, StoreError>;

    /// Boolean test for receive-path gating. Replaces
    /// `src-tauri/src/state_helpers::is_friend`.
    /// Returns `true` only for `FriendStatus::Active`.
    async fn is_active_friend(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<bool, StoreError> {
        Ok(self
            .lookup_by_pubkey(owner_key, pubkey_hex)
            .await?
            .is_some_and(|f| f.status == FriendStatus::Active))
    }
}
