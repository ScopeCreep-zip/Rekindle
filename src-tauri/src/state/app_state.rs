//! Phase 23.B — `AppState` struct + `Default` + small `impl` block
//! lifted out of `state.rs`. Consumers continue to import via
//! `crate::state::AppState` thanks to the `pub use` re-export at the
//! state-module root.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_gossip::dedup::DedupCache;
use tokio::sync::mpsc;

use rekindle_channel::AutoModCompiledCache;
use super::circuit::CircuitBreakerState;
use super::community::CommunityState;
use super::friend::{FriendState, IdentityState, UserStatus};
use super::runtime::{
    DHTManagerHandle, GameDetectorHandle, NodeHandle, RoutingManagerHandle, SignalManagerHandle,
    VoiceEngineHandle,
};

/// Central application state shared across all Tauri commands and services.
pub struct AppState {
    /// Channel for routing incoming voice packets from the dispatch loop to the receive loop.
    pub voice_packet_tx: Arc<RwLock<Option<mpsc::Sender<rekindle_voice::transport::VoicePacket>>>>,
    /// Identity (loaded after Stronghold unlock).
    pub identity: Arc<RwLock<Option<IdentityState>>>,
    /// Friends list with presence info.
    pub friends: Arc<RwLock<HashMap<String, FriendState>>>,
    /// Joined communities.
    pub communities: Arc<RwLock<HashMap<String, CommunityState>>>,
    /// Veilid node handle (set after login/attach).
    pub node: Arc<RwLock<Option<NodeHandle>>>,
    /// DHT record manager for reading/writing distributed state.
    pub dht_manager: Arc<RwLock<Option<DHTManagerHandle>>>,
    /// Routing manager for private route lifecycle.
    pub routing_manager: Arc<RwLock<Option<RoutingManagerHandle>>>,
    /// W16.9b — outbound-only `TransportNode` adopted against the host's
    /// running VeilidAPI. Send paths in transport's `operations::*` call
    /// into this; receive paths go through existing src-tauri services
    /// until those services migrate flow-by-flow.
    pub transport: Arc<RwLock<Option<Arc<rekindle_transport::TransportNode>>>>,
    /// W16.9b — transport's `Session` mirroring src-tauri's identity +
    /// friend-inbox metadata. Populated at login. Shared with the
    /// transport's route_refresh_loop so community routes refresh
    /// alongside the personal route. `None` before login.
    pub transport_session:
        Arc<parking_lot::RwLock<Option<rekindle_transport::session::Session>>>,
    /// W16.9b — durable retry queue store. Used by `EnvelopeQueue` for
    /// DM body sends + future expect-reply flows. `Some` after app
    /// startup wires `SqliteEnvelopeStore`.
    pub envelope_store: Arc<RwLock<Option<Arc<dyn rekindle_transport::EnvelopeStore>>>>,
    /// Phase 2 Track A — Receive-path friend authority. SQLite-backed
    /// (`SqliteFriendStore`) wired at app setup so the Veilid dispatch
    /// loop can authorize inbound envelopes against the source-of-truth
    /// friends table — no in-memory cache, no hydration race.
    pub friend_store: Arc<RwLock<Option<Arc<dyn rekindle_transport::FriendStore>>>>,
    /// Signal session manager (set after identity unlock). `RwLock` so
    /// encrypt/decrypt readers don't serialize on the wrapper; the
    /// inner `Arc<SignalManagerHandle>` is cloned out under the
    /// read-guard and the guard is dropped before any `.await`.
    pub signal_manager: Arc<RwLock<Option<Arc<SignalManagerHandle>>>>,
    /// Game detector state.
    pub game_detector: Arc<Mutex<Option<GameDetectorHandle>>>,
    /// Voice engine state. `pub(crate)` so the Tauri-runtime adapters
    /// keep direct access while external callers go through the typed
    /// `AppState` helpers below.
    pub(crate) voice_engine: Arc<Mutex<Option<VoiceEngineHandle>>>,
    /// Channel for sending shutdown signals to background services.
    pub shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    /// Channel for sending shutdown signal to the sync service.
    pub sync_shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    /// Sender half of the watch channel that tracks Veilid public internet readiness.
    pub network_ready_tx: Arc<tokio::sync::watch::Sender<bool>>,
    /// Receiver half — clone and `.changed().await` to wait for readiness.
    pub network_ready_rx: tokio::sync::watch::Receiver<bool>,
    /// Ed25519 secret key bytes for signing `MessageEnvelope`s. Stored
    /// after identity unlock so `message_service` can sign outgoing messages.
    pub identity_secret: Mutex<Option<[u8; 32]>>,
    /// Phase 4 — tamper-evident audit chain. `None` until vault unlock
    /// runs `audit_repo::restore_chain`.
    pub audit_chain: Mutex<Option<rekindle_audit::AuditChain>>,
    /// Phase 4 — shared keystore handle so background services that
    /// only have `&AppState` can reach the vault.
    pub keystore: crate::keystore::KeystoreHandle,
    /// Phase 5 — 9-state lifecycle FSM.
    pub lifecycle: std::sync::Arc<rekindle_lifecycle::AppLifecycle>,
    /// Phase 7 — friendship inbox-scan handle (direct_tx + shutdown_tx +
    /// watch_disabled_until + watch_tx + watch_counter consolidated).
    pub friendship_handle: std::sync::Arc<crate::services::friendship::FriendshipHandle>,
    /// Phase 8 — shared idempotency cache for mutating commands
    /// returning `Result<(), String>`.
    pub idempotency:
        std::sync::Arc<rekindle_idempotency::IdempotencyCache<Result<(), String>>>,
    /// Phase 8 — second cache for commands returning a string id
    /// (e.g. `create_channel`). Generics force one cache per `T`.
    pub idempotency_string:
        std::sync::Arc<rekindle_idempotency::IdempotencyCache<Result<String, String>>>,
    /// Phase 9 — cold-start buffer for Veilid updates that arrive before
    /// the app has the state needed to process them.
    pub cold_start: std::sync::Arc<
        rekindle_transport::subscriptions::ColdStartBuffer<veilid_core::VeilidUpdate>,
    >,
    /// Phase 10 — in-memory journal of Tauri-emitted events. Capacity
    /// 10_000, FIFO eviction. Wired by `event_dispatch::emit_journaled`.
    pub event_journal:
        std::sync::Arc<rekindle_events::EventJournal<crate::event_dispatch::TauriEmitRecord>>,
    /// Phase 23.A — single-source event-emission router. Every emit
    /// (live or journaled) pushes through this mpsc queue.
    pub event_dispatch: std::sync::Arc<crate::event_dispatch::EventDispatch>,
    /// Phase 10 — high-watermark of journal entries already replayed
    /// by `event_resume`.
    pub event_replay_watermark: parking_lot::Mutex<u64>,
    /// Handles for spawned background tasks. Aborted on logout.
    pub background_handles: Mutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
    /// Legacy community-level MEK cache: `community_id` → `MediaEncryptionKey`.
    pub mek_cache: Mutex<HashMap<String, MediaEncryptionKey>>,
    /// Per-channel MEK cache: `(community_id, channel_id)` → `MediaEncryptionKey`.
    pub channel_mek_cache: Mutex<HashMap<(String, String), MediaEncryptionKey>>,
    /// Friends whose DHT `watch_dht_values` returned false. The sync
    /// service uses `force_refresh=true` for these friends.
    pub unwatched_friends: RwLock<HashSet<String>>,
    /// `JoinHandle` for the Veilid dispatch loop.
    pub dispatch_loop_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Shutdown sender for the route refresh loop.
    pub route_refresh_shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
    /// Shutdown sender for the idle/auto-away service.
    pub idle_shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
    /// Shutdown sender for the presence heartbeat loop.
    pub heartbeat_shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
    /// The status the user had before auto-away kicked in.
    pub pre_away_status: RwLock<Option<UserStatus>>,
    /// Deep link action received before authentication; replayed after login.
    pub pending_deep_link: Mutex<Option<crate::deep_links::DeepLinkAction>>,
    /// Per-community circuit breaker for remote Veilid RPCs.
    pub community_circuit_breakers: RwLock<HashMap<String, CircuitBreakerState>>,
    /// Tauri app handle — set during `.setup()`.
    pub app_handle: RwLock<Option<tauri::AppHandle>>,
    /// Global dedup cache for gossip mesh message deduplication.
    pub dedup_cache: Mutex<DedupCache>,
    /// M10.4 — receiver-side per-(community, sender) gossip rate floor.
    pub gossip_rate_limits:
        Mutex<HashMap<(String, String), rekindle_gossip::rate_limit::TokenBucket>>,
    /// M10.4 — receiver-side per-(community, channel, sender) last-send
    /// timestamps (unix seconds) for slowmode enforcement.
    pub channel_last_received: Mutex<HashMap<(String, String, String), u64>>,
    /// Wave 7 P7.3 — per-relay circuit-breaker state.
    pub relay_health: Mutex<
        HashMap<
            crate::services::relay::health::RelayKey,
            crate::services::relay::health::RelayHealth,
        >,
    >,
    /// Sender for queued SMPL channel-message writes.
    pub channel_write_retry_tx:
        Arc<RwLock<Option<rekindle_records::retry::WriteQueueHandle>>>,
    /// Per-community compiled AutoMod cache.
    pub automod_cache: Arc<RwLock<HashMap<String, Arc<AutoModCompiledCache>>>>,
    /// Wake-up signal for the event reminder scheduler.
    pub event_reminder_wake_tx: Arc<RwLock<Option<tokio::sync::watch::Sender<u64>>>>,
    /// Per-community Lost Cargo chunk caches.
    pub file_caches: RwLock<HashMap<String, rekindle_files::ChunkCache>>,
    /// Pinned-attachment IDs per community.
    pub pinned_attachments: RwLock<HashMap<String, rekindle_files::PinnedSet>>,
    /// Root directory for all per-community Lost Cargo caches.
    pub file_cache_root: RwLock<Option<std::path::PathBuf>>,
    /// DM MEK cache (architecture §27.1). SMPL record key → `DmMekChain`.
    pub dm_mek_cache: Mutex<HashMap<String, rekindle_dm::DmMekChain>>,
    /// Strand Relay probe cooldown.
    pub relay_probe_cooldown: Mutex<HashMap<String, u64>>,
    /// Mutual Aid §14.5 dirty set for per-peer reliability counters.
    pub relay_reliability_dirty: Mutex<HashSet<(String, String)>>,
    /// Push relay (architecture §17.3 Tier 3) wake-notify debounce.
    pub last_wake_notify_secs: Mutex<u64>,
    /// Architecture §10.6 video / screen-share reassembly.
    pub video_reassembly: rekindle_video::VideoReassemblyState,
    /// W11.4 — per-peer DM video reassembly state.
    pub dm_video_reassembly: crate::services::dm::video::DmVideoReassemblyState,
    /// Architecture §17.2 line 2402 — per-channel notification burst throttle.
    pub notification_throttle:
        crate::services::community::notifications::NotificationThrottle,
    /// Phase 14.q — 1:1 call registry, wrapped behind `CallRegistry` trait.
    pub active_calls: Arc<dyn rekindle_calls::signaling::CallRegistry>,
    /// Wave 12 W12.12 — temporarily muted peers (peer_pubkey_hex →
    /// expires_at_ms). In-memory only.
    pub temp_call_muted: Arc<Mutex<HashMap<String, u64>>>,
    /// Wave 14 W14.4 — observability counter for dropped voice packets.
    pub voice_pkt_drops: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Wave 14 W14.1 — pre-staged voice packet receiver.
    pub voice_packet_rx_staged: parking_lot::Mutex<
        Option<tokio::sync::mpsc::Receiver<rekindle_voice::transport::VoicePacket>>,
    >,
    /// Wave 12 W12.9 — group call state (1:N).
    pub group_calls:
        Arc<Mutex<HashMap<String, rekindle_calls::group_state::GroupCallState>>>,
    /// P3.3 — incoming SessionResetRequest payloads awaiting user confirmation.
    /// Held in memory only — never persisted before user explicit consent.
    pub pending_session_resets: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl Default for AppState {
    fn default() -> Self {
        let (network_ready_tx, network_ready_rx) = tokio::sync::watch::channel(false);
        Self {
            voice_packet_tx: Arc::new(RwLock::new(None)),
            identity: Arc::new(RwLock::new(None)),
            friends: Arc::new(RwLock::new(HashMap::new())),
            communities: Arc::new(RwLock::new(HashMap::new())),
            node: Arc::new(RwLock::new(None)),
            dht_manager: Arc::new(RwLock::new(None)),
            routing_manager: Arc::new(RwLock::new(None)),
            transport: Arc::new(RwLock::new(None)),
            transport_session: Arc::new(parking_lot::RwLock::new(None)),
            envelope_store: Arc::new(RwLock::new(None)),
            friend_store: Arc::new(RwLock::new(None)),
            signal_manager: Arc::new(RwLock::new(None)),
            game_detector: Arc::new(Mutex::new(None)),
            voice_engine: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            sync_shutdown_tx: Arc::new(RwLock::new(None)),
            network_ready_tx: Arc::new(network_ready_tx),
            network_ready_rx,
            identity_secret: Mutex::new(None),
            audit_chain: Mutex::new(None),
            keystore: crate::keystore::new_handle(),
            lifecycle: std::sync::Arc::new(rekindle_lifecycle::AppLifecycle::new()),
            friendship_handle: crate::services::friendship::FriendshipHandle::new(),
            idempotency: std::sync::Arc::new(
                rekindle_idempotency::IdempotencyCache::with_defaults(),
            ),
            idempotency_string: std::sync::Arc::new(
                rekindle_idempotency::IdempotencyCache::with_defaults(),
            ),
            cold_start: std::sync::Arc::new(
                rekindle_transport::subscriptions::ColdStartBuffer::new(),
            ),
            event_journal: std::sync::Arc::new(rekindle_events::EventJournal::new(10_000)),
            event_dispatch: std::sync::Arc::new(crate::event_dispatch::EventDispatch::new()),
            event_replay_watermark: parking_lot::Mutex::new(0),
            background_handles: Mutex::new(Vec::new()),
            mek_cache: Mutex::new(HashMap::new()),
            channel_mek_cache: Mutex::new(HashMap::new()),
            unwatched_friends: RwLock::new(HashSet::new()),
            dispatch_loop_handle: RwLock::new(None),
            route_refresh_shutdown_tx: RwLock::new(None),
            idle_shutdown_tx: RwLock::new(None),
            heartbeat_shutdown_tx: RwLock::new(None),
            pre_away_status: RwLock::new(None),
            pending_deep_link: Mutex::new(None),
            community_circuit_breakers: RwLock::new(HashMap::new()),
            app_handle: RwLock::new(None),
            dedup_cache: Mutex::new(DedupCache::new(1024)),
            gossip_rate_limits: Mutex::new(HashMap::new()),
            channel_last_received: Mutex::new(HashMap::new()),
            relay_health: Mutex::new(HashMap::new()),
            channel_write_retry_tx: Arc::new(RwLock::new(None)),
            automod_cache: Arc::new(RwLock::new(HashMap::new())),
            event_reminder_wake_tx: Arc::new(RwLock::new(None)),
            file_caches: RwLock::new(HashMap::new()),
            pinned_attachments: RwLock::new(HashMap::new()),
            file_cache_root: RwLock::new(None),
            dm_mek_cache: Mutex::new(HashMap::new()),
            relay_probe_cooldown: Mutex::new(HashMap::new()),
            relay_reliability_dirty: Mutex::new(HashSet::new()),
            last_wake_notify_secs: Mutex::new(0),
            video_reassembly: rekindle_video::VideoReassemblyState::new(),
            dm_video_reassembly: crate::services::dm::video::DmVideoReassemblyState::new(),
            notification_throttle:
                crate::services::community::notifications::NotificationThrottle::new(),
            active_calls: Arc::new(
                crate::services::calls_adapter::ActiveCallRegistry::new(Arc::new(Mutex::new(
                    HashMap::new(),
                ))),
            ),
            temp_call_muted: Arc::new(Mutex::new(HashMap::new())),
            voice_pkt_drops: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            voice_packet_rx_staged: parking_lot::Mutex::new(None),
            group_calls: Arc::new(Mutex::new(HashMap::new())),
            pending_session_resets: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl AppState {
    /// Toggle the voice engine's mute state. Returns `Some(new_muted)` if
    /// the voice engine is active, `None` otherwise. The caller is
    /// responsible for emitting any UI event tied to the change.
    pub fn toggle_voice_mute(&self) -> Option<bool> {
        let mut ve = self.voice_engine.lock();
        let handle = ve.as_mut()?;
        let new_muted = !handle.engine.is_muted;
        handle.engine.set_muted(new_muted);
        handle
            .muted_flag
            .store(new_muted, std::sync::atomic::Ordering::Relaxed);
        Some(new_muted)
    }

    /// Force the voice engine into muted=true. Used by the §10.7 stage
    /// audience auto-mute gate. No-op when no voice engine is active.
    pub fn force_voice_mute(&self) {
        let mut ve = self.voice_engine.lock();
        if let Some(handle) = ve.as_mut() {
            handle.engine.set_muted(true);
            handle
                .muted_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Snapshot the voice transport's peer keys when the active engine is
    /// joined to the given community + channel. Returns an empty Vec when
    /// no engine is active or it is joined to a different channel. Used
    /// by §10.5 MEK rotation to enumerate recipients of new-generation
    /// keys.
    pub fn voice_engine_peer_keys_for_channel(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Vec<String> {
        let ve = self.voice_engine.lock();
        let Some(handle) = ve.as_ref() else {
            return Vec::new();
        };
        if handle.community_id.as_deref() != Some(community_id)
            || handle.channel_id != channel_id
        {
            return Vec::new();
        }
        let transport = handle.transport.blocking_lock();
        transport.peer_keys()
    }
}
