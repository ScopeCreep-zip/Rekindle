//! Phase 21 REDO — `FriendPresenceDeps` composite trait + its DTOs.
//!
//! Bag of operations the friend-presence orchestrators (DHT
//! value-change dispatch, `watch_friend`, `publish_status`,
//! `start_heartbeat_loop`) need from their host. Implemented in
//! src-tauri by `PresenceAdapter` against the live `AppState` +
//! `AppHandle` + `DbPool`.

use async_trait::async_trait;

use crate::deps::PresenceError;
use crate::status::UserStatusKind;

/// Subset of the in-process game-presence record the friend-presence
/// path consumes. Kept here so the crate stays free of src-tauri
/// types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInfoSnapshot {
    pub game_id: u32,
    pub game_name: String,
    pub elapsed_seconds: u32,
    pub server_address: Option<String>,
}

/// Result of swapping a friend's status. The orchestrator uses
/// `was_offline` to decide whether to emit a one-shot `FriendOnline`
/// event in addition to the rolling `StatusChanged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetFriendStatusOutcome {
    pub was_offline: bool,
    pub friend_existed: bool,
}

/// Events the orchestrators emit. The adapter maps each variant to
/// the matching src-tauri `PresenceEvent` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FriendPresenceEvent {
    FriendOnline {
        friend_key: String,
    },
    FriendOffline {
        friend_key: String,
    },
    StatusChanged {
        friend_key: String,
        status: UserStatusKind,
    },
    GameChanged {
        friend_key: String,
        game: Option<GameInfoSnapshot>,
    },
}

/// Composite Deps for every friend-presence op.
///
/// All veilid-typed concerns (RecordKey parsing, RoutingContext
/// acquisition, watch subscriptions, set_dht_value) are hidden behind
/// adapter-side methods that exchange only strings + byte vectors.
#[async_trait]
pub trait FriendPresenceDeps: Send + Sync + 'static {
    // === Friend lookup ===
    fn friend_for_dht_key(&self, dht_key: &str) -> Option<String>;
    fn is_friend_accepted(&self, friend_key: &str) -> bool;

    // === Friend state mutations ===
    /// Apply a status update and return whether the prior status was
    /// `Offline` (so the caller can fire a `FriendOnline` edge event).
    fn set_friend_status(&self, friend_key: &str, status: UserStatusKind)
        -> SetFriendStatusOutcome;
    fn set_friend_offline(&self, friend_key: &str, last_seen_ts_ms: i64);
    fn set_friend_last_heartbeat(&self, friend_key: &str, heartbeat_ts_ms: i64);
    fn set_friend_game_info(&self, friend_key: &str, game: Option<GameInfoSnapshot>);
    fn set_friend_dht_record_key(&self, friend_key: &str, dht_record_key: &str);

    // === DHT manager mutations ===
    fn register_friend_dht_key(&self, dht_key: &str, friend_key: &str);
    fn cache_route_blob(&self, friend_key: &str, blob: Vec<u8>);
    fn track_open_record(&self, dht_record_key: &str);
    fn set_unwatched_friend(&self, friend_key: &str, unwatched: bool);

    // === DHT IO (async) ===
    /// Open a friend's DHT record for read. Returns `Ok` even if the
    /// open fails — non-fatal (the friend will sync on the next
    /// interval). On hard failures (invalid key) returns `Err`.
    async fn open_friend_record(&self, dht_record_key: &str) -> Result<(), PresenceError>;
    /// Subscribe to subkey changes. Returns `Ok(true)` if a live watch
    /// is established, `Ok(false)` if Veilid couldn't set up the watch
    /// (caller falls back to polling), `Err` on hard errors.
    async fn watch_friend_subkeys(
        &self,
        dht_record_key: &str,
        subkeys: &[u32],
    ) -> Result<bool, PresenceError>;

    /// Returns `(profile_dht_key, owner_keypair_str)` from the live
    /// node handle, or `None` when the node isn't ready.
    fn profile_dht_info(&self) -> Option<(String, Option<String>)>;
    /// Open the profile record for write using the supplied owner
    /// keypair (string form). Idempotent — Veilid treats reopens of an
    /// already-open record as a no-op.
    async fn open_profile_record_for_write(
        &self,
        profile_key: &str,
        owner_keypair_str: Option<&str>,
    ) -> Result<(), PresenceError>;
    /// Write the 9-byte `[status_byte, timestamp_be]` payload to
    /// profile subkey 2.
    async fn write_profile_status_subkey(
        &self,
        profile_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), PresenceError>;

    // === Persistence ===
    fn persist_friend_last_seen(&self, friend_key: &str, ts_ms: i64);

    // === Identity ===
    /// Current authenticated user's status (if logged in).
    fn current_identity_status(&self) -> Option<UserStatusKind>;

    /// Wall-clock now in milliseconds since the unix epoch. Hoisted
    /// onto the trait so tests can pin time without monkeying with
    /// `std::time`.
    fn now_ms(&self) -> i64;

    // === Event emit ===
    fn emit(&self, event: FriendPresenceEvent);

    // === Friend-sync surface (22.c-REDO) ===

    /// `(friend_key, dht_record_key)` pairs for every friend whose
    /// profile DHT record is known. Used by the periodic sync loop
    /// to iterate friends + force-poll their profile subkeys when
    /// watches haven't fired.
    fn friends_with_dht_keys(&self) -> Vec<(String, String)>;

    /// Friend keys whose DHT watch failed and need force-polling
    /// from the network each tick (per Veilid GitLab #377). Pre-port
    /// these landed in `state.unwatched_friends`.
    fn unwatched_friends(&self) -> std::collections::HashSet<String>;

    /// Force a fresh `get_dht_value` for one subkey on the friend's
    /// profile record. Returns the raw bytes when the subkey has
    /// a payload, `None` when it's empty or the read failed.
    async fn fetch_friend_dht_subkey(
        &self,
        dht_record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Option<Vec<u8>>;

    /// Friend keys whose `last_heartbeat_at` is older than
    /// `threshold_ms` (and aren't already Offline). The sync loop
    /// marks each one offline + emits FriendOffline (privacy-gated
    /// by `is_friend_accepted`). Pre-port this lived in
    /// `check_stale_presences`.
    fn find_stale_friend_heartbeats(&self, threshold_ms: i64) -> Vec<String>;
}
