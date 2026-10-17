//! Receive-path friend authority: "is this envelope from a known active
//! friend?"
//!
//! The trait is Tier 1 so every host shares one shape. The SQLite
//! implementation lives in `rekindle-db` (`SqliteFriendStore`). Today the
//! desktop's receive path consults it through
//! `state_helpers::friend_status_authoritative`; the daemon wires the
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

/// A friendship's state: the one type every host uses, stored in
/// `friends.friendship_state` (plan D2).
///
/// Authorization rule applied at receive-dispatch:
/// - `Accepted`: the sender is authorized for any envelope variant.
/// - `PendingOut`: *we* sent a friend request and are awaiting their
///   accept. The sender is authorized only for `FriendAccept` (W16.10e
///   idempotent receive; enforced in the desktop's
///   `dispatch::prepare::sender_admitted`, plan C7.25); other variants
///   drop.
/// - `Removing`: we removed the friend and the `Unfriended` notice is still
///   queued to them. The row keeps their routing information for the
///   retries and is forgotten once nothing is queued; their envelopes drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FriendshipState {
    PendingOut,
    Accepted,
    Removing,
}

impl FriendshipState {
    /// The stored form (`friends.friendship_state`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingOut => "pending_out",
            Self::Accepted => "accepted",
            Self::Removing => "removing",
        }
    }
}

/// A stored friendship state that is none of the known values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown friendship state {0:?}")]
pub struct UnknownFriendshipState(pub String);

impl std::str::FromStr for FriendshipState {
    type Err = UnknownFriendshipState;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending_out" => Ok(Self::PendingOut),
            "accepted" => Ok(Self::Accepted),
            "removing" => Ok(Self::Removing),
            other => Err(UnknownFriendshipState(other.to_string())),
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
    pub status: FriendshipState,
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

    /// All `Accepted` friends for this owner. Used at attach time to install
    /// per-pair inbox watches for each.
    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, FriendStoreError>;

    /// The sender's friendship status, or `None` for a stranger. The
    /// receive gate reads it: `FriendAccept` is admitted from a
    /// `PendingOut` sender, everything else needs `Accepted` (plan C7.25).
    async fn status_of(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<Option<FriendshipState>, FriendStoreError> {
        Ok(self
            .lookup_by_pubkey(owner_key, pubkey_hex)
            .await?
            .map(|f| f.status))
    }

    /// Boolean test for receive-path gating. Replaces
    /// `src-tauri/src/state_helpers::is_friend`.
    /// Returns `true` only for `FriendshipState::Accepted`.
    async fn is_active_friend(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<bool, FriendStoreError> {
        Ok(self
            .lookup_by_pubkey(owner_key, pubkey_hex)
            .await?
            .is_some_and(|f| f.status == FriendshipState::Accepted))
    }
}
