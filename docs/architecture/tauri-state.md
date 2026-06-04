# AppState Reference

This is the field-by-field reference for `src-tauri/src/state/app_state.rs`.
`AppState` holds every long-lived runtime concern of the desktop app and is
shared across commands and services as `SharedState = Arc<AppState>`.

`state.rs` itself is now a thin module re-export; the struct lives in
`state/app_state.rs` and helper types are split across `state/{circuit,
community, friend, gossip, runtime}.rs`. Read-only accessors live in
`state_helpers/` (see the [Read-only accessors](#read-only-accessors)
section at the bottom).

There are ~47 fields. They are grouped below by concern, not by type.

## Identity, Veilid, and Lifecycle

| Field | Type | Purpose |
|---|---|---|
| `identity` | `Arc<RwLock<Option<IdentityState>>>` | Logged-in user identity, set after vault unlock |
| `identity_secret` | `Mutex<Option<[u8; 32]>>` | Ed25519 secret bytes for envelope signing |
| `signal_manager` | `Arc<RwLock<Option<Arc<SignalManagerHandle>>>>` | Signal session manager (X3DH + Double Ratchet) |
| `keystore` | `crate::keystore::KeystoreHandle` | VaultStore handle (`rekindle-vault` SQLCipher) |
| `node` | `Arc<RwLock<Option<NodeHandle>>>` | Veilid node handle |
| `dht_manager` | `Arc<RwLock<Option<DHTManagerHandle>>>` | DHT record manager |
| `routing_manager` | `Arc<RwLock<Option<RoutingManagerHandle>>>` | Private route lifecycle |
| `transport` | `Arc<RwLock<Option<Arc<rekindle_transport::TransportNode>>>>` | Outbound-only `TransportNode` (Wave 16.9b — Tauri-side adoption of the daemon's transport boundary) |
| `transport_session` | `Arc<parking_lot::RwLock<Option<Session>>>` | Session mirror used by route-refresh + community route refresh |
| `lifecycle` | `Arc<rekindle_lifecycle::AppLifecycle>` | 9-state FSM gating mutating commands via `TransportGuard` |
| `network_ready_tx` / `_rx` | `tokio::sync::watch::{Sender,Receiver}<bool>` | Public-internet readiness for the dispatch loop |
| `dispatch_loop_handle` | `RwLock<Option<JoinHandle<()>>>` | Veilid dispatch loop task handle |
| `app_handle` | `RwLock<Option<tauri::AppHandle>>` | Tauri AppHandle, set during `setup()` |
| `cold_start` | `Arc<…>` | Cold-start buffer for updates that arrive before the dispatch loop is hot |
| `pending_deep_link` | `Mutex<Option<DeepLinkAction>>` | Deep link received pre-auth (replayed post-login) |

## Persistent Stores

| Field | Type | Purpose |
|---|---|---|
| `audit_chain` | `Mutex<Option<rekindle_audit::AuditChain>>` | Tamper-evident BLAKE3-keyed audit chain. MAC key lives in the vault under `("audit", "mac_key")`. See [`audit-chain.md`](audit-chain.md). |
| `envelope_store` | `Arc<RwLock<Option<Arc<dyn rekindle_transport::EnvelopeStore>>>>` | SQLite-backed durable pending-envelope retry queue |
| `friend_store` | `Arc<RwLock<Option<Arc<dyn rekindle_transport::FriendStore>>>>` | SQLite-backed friend authority used by `rekindle-transport`'s receive path |
| `channel_write_retry_tx` | `Arc<RwLock<Option<rekindle_records::retry::WriteQueueHandle>>>` | Queued SMPL channel-message write handle |

## Friends, Friendship, Presence

| Field | Type | Purpose |
|---|---|---|
| `friends` | `Arc<RwLock<HashMap<String, FriendState>>>` | Friend state by pubkey |
| `unwatched_friends` | `RwLock<HashSet<String>>` | Friends whose DHT watch failed; `sync_service` polls them with `force_refresh=true` |
| `pre_away_status` | `RwLock<Option<UserStatus>>` | Status before auto-away (restored on activity) |
| `friendship_handle` | `Arc<FriendshipHandle>` | `rekindle-friendship::InboxScanCoordinator` wiring — watch + 30 s poll + direct trigger |
| `pending_session_resets` | `Arc<Mutex<HashMap<String, Vec<u8>>>>` | In-memory `SessionResetRequest` map (resync after Signal session reset) |

## Communities & Channels

| Field | Type | Purpose |
|---|---|---|
| `communities` | `Arc<RwLock<HashMap<String, CommunityState>>>` | Community state by community ID |
| `mek_cache` | `Mutex<HashMap<String, MediaEncryptionKey>>` | Legacy community-level MEK cache |
| `channel_mek_cache` | `Mutex<HashMap<(String, String), MediaEncryptionKey>>` | Per-channel MEK cache, keyed by `(community_id, channel_id)` |
| `community_circuit_breakers` | `RwLock<HashMap<String, CircuitBreakerState>>` | Per-community RPC circuit breaker (3 fails → 30 s cooldown) |
| `automod_cache` | `Arc<RwLock<HashMap<String, Arc<AutoModCompiledCache>>>>` | Compiled regex cache for AutoMod rules |
| `event_reminder_wake_tx` | `Arc<RwLock<Option<tokio::sync::watch::Sender<u64>>>>` | Wake signal for the event reminder scheduler |
| `notification_throttle` | `…NotificationThrottle` | Architecture §17.2 per-channel burst throttle |
| `gossip_rate_limits` | `…` | Per-(community, sender) token-bucket rate limiter |
| `channel_last_received` | `Mutex<HashMap<(String, String, String), u64>>` | Per-(community, channel, sender) slowmode timestamps |

## DMs and Calls

| Field | Type | Purpose |
|---|---|---|
| `dm_mek_cache` | `Mutex<HashMap<String, rekindle_dm::DmMekChain>>` | DM MEK chain — genesis plus every materialised generation |
| `active_calls` | `Arc<dyn rekindle_calls::signaling::CallRegistry>` | Phase 14.q call registry trait object — every chat / DM / community call goes through this surface |
| `group_calls` | `Arc<Mutex<HashMap<String, GroupCallState>>>` | Group call state by group ID |
| `temp_call_muted` | `Arc<Mutex<HashMap<String, u64>>>` | Temporarily muted peers (in-memory only) |

## Voice, Video, Files

| Field | Type | Purpose |
|---|---|---|
| `voice_engine` | `VoiceEngineHandle` | Engine + transport + send / recv / MCU / device-monitor task handles |
| `voice_packet_tx` | `Arc<RwLock<Option<mpsc::Sender<VoicePacket>>>>` | Routes inbound voice packets from dispatch loop to receive loop |
| `voice_packet_rx_staged` | `parking_lot::Mutex<…>` | Pre-staged voice receiver hot before the call window opens |
| `voice_pkt_drops` | `Arc<AtomicU64>` | Counter for dropped voice packets (telemetry) |
| `video_reassembly` | `rekindle_video::VideoReassemblyState` | Per-community video / screen-share reassembly buffers |
| `dm_video_reassembly` | `…DmVideoReassemblyState` | Per-peer DM video reassembly |
| `video_channels` | `crate::video_channels::VideoChannelRegistry` | Phase 11 Tier 1 high-throughput video channel registry; frames bypass `event_dispatch` |
| `game_detector` | `Arc<Mutex<Option<GameDetectorHandle>>>` | Game detection state |
| `file_caches` | `RwLock<HashMap<String, ChunkCache>>` | Per-community Lost Cargo `ChunkCache` |
| `pinned_attachments` | `RwLock<HashMap<String, PinnedSet>>` | Per-community pinned attachment IDs (skipped during eviction) |
| `file_cache_root` | `RwLock<Option<PathBuf>>` | `<app_data>/file_cache/` root path |

## Relay, Push Relay, Notifications

| Field | Type | Purpose |
|---|---|---|
| `relay_health` | `…` | Per-relay circuit breaker |
| `relay_probe_cooldown` | `Mutex<HashMap<String, u64>>` | Strand Relay status-probe cooldown |
| `relay_reliability_dirty` | `Mutex<HashSet<(String, String)>>` | Mutual Aid §14.5 dirty set (flushed to SQLite every 30 s) |
| `last_wake_notify_secs` | `Mutex<u64>` | Push-relay wake-notify debounce |

## Event Dispatch and Journal (Phase 23.A)

| Field | Type | Purpose |
|---|---|---|
| `event_dispatch` | `Arc<crate::event_dispatch::EventDispatch>` | Single-source emit router for every Rust → Frontend event. See [`event-dispatch.md`](event-dispatch.md). |
| `event_journal` | `…` | 10 k-capacity FIFO event log for Phase 10 reconnect replay (`rekindle-events::EventJournal`) |
| `event_replay_watermark` | `parking_lot::Mutex<u64>` | High-water mark for resumed events |
| `dedup_cache` | `Mutex<DedupCache>` | Global gossip-mesh dedup cache |

## Idempotency

| Field | Type | Purpose |
|---|---|---|
| `idempotency` | `Arc<IdempotencyCache<Result<(), String>>>` | LRU + TTL cache for `Result<(), String>`-returning mutating commands |
| `idempotency_string` | `Arc<IdempotencyCache<Result<String, String>>>` | LRU + TTL cache for `Result<String, String>`-returning commands |

## Shutdown Channels

`mpsc::Sender<()>` for graceful shutdown of the corresponding background
service: `shutdown_tx`, `sync_shutdown_tx`,
`route_refresh_shutdown_tx`, `idle_shutdown_tx`,
`heartbeat_shutdown_tx`. `background_handles` holds the
`Vec<tauri::async_runtime::JoinHandle<()>>` aborted on logout.

## `state/` submodules

`state/` is a directory; helper types are split by concern:

| File | Contents |
|---|---|
| `state/app_state.rs` | `AppState` struct definition + constructors |
| `state/circuit.rs` | `CircuitBreakerState`, transition logic |
| `state/community.rs` | `CommunityState`, member-cache types |
| `state/friend.rs` | `FriendState`, presence sub-state |
| `state/gossip.rs` | `DedupCache`, `TokenBucket`, gossip rate-limit types |
| `state/runtime.rs` | Shutdown signals, background handle types, runtime-only glue |

## Read-only Accessors

`state_helpers/` houses concern-grouped read-only accessor functions so
command handlers do not need to know about lock acquisition. Reading a
value goes through these helpers; writing still touches `AppState`
fields directly via the appropriate runtime/adapter.

| Module | Purpose |
|---|---|
| `state_helpers/identity.rs` | Identity lookups, pubkey / display-name / device |
| `state_helpers/friends.rs` | Friend lookup, status, presence |
| `state_helpers/communities.rs` | Community lookup, member roles, channel access checks |
| `state_helpers/governance.rs` | `GovernanceState` snapshot read |
| `state_helpers/governance_persist.rs` | Hydrated governance-cache writes (paired with reads) |
| `state_helpers/dht_records.rs` | Cached DHT record key lookups |
| `state_helpers/node.rs` | `NodeHandle` / `DHTManagerHandle` / `RoutingManagerHandle` access |
| `state_helpers/routes.rs` | Private route state |
| `state_helpers/circuit_breaker.rs` | Circuit-breaker query helpers |

The pattern is:

1. Acquire the relevant `Arc<RwLock<…>>` field via the helper.
2. Clone the data out (because parking_lot guards are `!Send` and
   cannot cross `.await` points).
3. Drop the guard before any `.await`.

`db_helpers::{db_call, db_call_or_default, db_fire}` use the same
discipline for the SQLite pool — fire-and-forget vs. async-await vs.
return-default semantics are picked per call site.
