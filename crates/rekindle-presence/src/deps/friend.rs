//! Phase 21 REDO — `FriendPresenceDeps` composite trait + its DTOs.
//!
//! Bag of operations the friend-presence orchestrators (DHT
//! value-change dispatch, `watch_friend`, `publish_status`,
//! `start_heartbeat_loop`) need from their host. Implemented in
//! src-tauri by `PresenceAdapter` against the live `AppState` +
//! `AppHandle` + `Db`.

use async_trait::async_trait;
use rekindle_records::lease::LeaseId;

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
/// What publishing our own STATUS needs (plan C7.8c): the one publisher
/// per session runs over this, so a host that has no friend-presence
/// surface (the daemon) implements only it.
#[async_trait]
pub trait StatusPublisherDeps: Send + Sync + 'static {
    /// Our profile record key, or `None` before the profile is published.
    fn profile_dht_info(&self) -> Option<String>;
    /// Write the 9-byte `[status_byte, timestamp_be]` payload to
    /// profile subkey 2, through the session's writable profile lease.
    async fn write_profile_status_subkey(
        &self,
        profile_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), PresenceError>;
    /// Current authenticated user's status (if logged in).
    fn current_identity_status(&self) -> Option<UserStatusKind>;
    /// Wall-clock now in milliseconds since the unix epoch. Hoisted
    /// onto the trait so tests can pin time without monkeying with
    /// `std::time`.
    fn now_ms(&self) -> i64;
}

#[async_trait]
pub trait FriendPresenceDeps: StatusPublisherDeps {
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
    fn set_unwatched_friend(&self, friend_key: &str, unwatched: bool);

    // === DHT IO (async) ===
    //
    // Records are borrowed from the host's record pool (plan C7.5). Each
    // watched friend's profile is held for the session under one lease,
    // which carries the watch.

    /// Borrow a friend's profile record (read-only).
    async fn acquire_friend_record(&self, dht_record_key: &str) -> Result<LeaseId, PresenceError>;
    /// Hand a friend's lease to the host, which keeps it for the session
    /// and releases the one it held for that friend before (the new lease
    /// carries the watch).
    async fn hold_friend_record(&self, friend_key: &str, lease: LeaseId);
    /// Watch subkeys of the leased record. `Ok` means the desired watch
    /// was accepted locally, never that a node granted it (V20): a watch
    /// no node accepts shows up as a dead-watch update, which the pool
    /// re-arms.
    async fn watch_friend_subkeys(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<(), PresenceError>;

    // === Persistence ===
    fn persist_friend_last_seen(&self, friend_key: &str, ts_ms: i64);

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
