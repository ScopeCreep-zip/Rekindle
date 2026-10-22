//! Phase 23.B — `AppState` struct + `Default` + small `impl` block
//! lifted out of `state.rs`. Consumers continue to import via
//! `crate::state::AppState` thanks to the `pub use` re-export at the
//! state-module root.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_gossip::dedup::DedupCache;
use rekindle_types::channel_keys::KeyScope;
use tokio::sync::mpsc;

use super::community::CommunityMap;
use super::friend::IdentityState;
use super::runtime::{NodeHandle, VoiceEngineHandle};
use rekindle_channel::AutoModCompiledCache;

/// Central application state shared across all Tauri commands and services.
pub struct AppState {
    /// Channel for routing incoming voice packets from the dispatch loop to the receive loop.
    pub voice_packet_tx: Arc<RwLock<Option<mpsc::Sender<rekindle_voice::transport::VoicePacket>>>>,
    /// Channel for routing incoming RFC 3550 receiver reports from the
    /// dispatch loop to the **send** loop — the return direction of
    /// `voice_packet_tx`. Reports tell our sender what our audio looks
    /// like at each peer, which is the only way it can see loss at all:
    /// a successful `send()` says nothing about arrival.
    pub voice_report_tx:
        Arc<RwLock<Option<mpsc::Sender<rekindle_voice::receiver_report::VoiceReceiverReport>>>>,
    /// Identity (loaded after Stronghold unlock).
    pub identity: Arc<RwLock<Option<IdentityState>>>,
    /// Joined communities: the Host's one store (`HostStores::communities`,
    /// plan E3.1), shared through [`Self::host_stores`].
    pub communities: CommunityMap,
    /// Veilid node handle (set after login/attach).
    pub node: Arc<RwLock<Option<NodeHandle>>>,
    /// W16.9b — outbound-only `TransportNode` adopted against the host's
    /// running VeilidAPI. Send paths in transport's `operations::*` call
    /// into this; receive paths go through existing src-tauri services
    /// until those services migrate flow-by-flow.
    pub transport: Arc<RwLock<Option<Arc<rekindle_transport::TransportNode>>>>,
    /// The unlocked identity's session projection (`host::session`), the
    /// host's own (`HostStores::session`) and the adopted node's. `None`
    /// while locked.
    pub transport_session: Arc<parking_lot::RwLock<Option<rekindle_transport::session::Session>>>,
    /// The logged-in identity's database (plan C6). Every command and
    /// service reaches SQLite through `db.current()`. Shared (`Arc`) with
    /// the friend store, which resolves it per call and so never holds
    /// the database open past `clear` (plan C7.18).
    pub db: Arc<rekindle_db::DbHandle>,
    /// The one wrong-passphrase counter per identity (plan D1, U1.9):
    /// unlock and delete count on it, and a locked-out identity is refused
    /// before Argon2id runs.
    pub identity_attempts: Arc<rekindle_idempotency::attempts::AttemptLimiter<String>>,
    /// The unlocked identity's preferences (`services::preferences`; the
    /// defaults while locked).
    pub preferences: RwLock<rekindle_types::preferences::Preferences>,
    /// Voice engine state. `pub(crate)` so the Tauri-runtime adapters
    /// keep direct access while external callers go through the typed
    /// `AppState` helpers below.
    pub(crate) voice_engine: Arc<Mutex<Option<VoiceEngineHandle>>>,
    /// Channel for sending shutdown signals to background services.
    pub shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    /// The identity's Ed25519 secret, stored at unlock for the unported
    /// paths that sign or derive from it (communities, voice, sync).
    pub identity_secret: Mutex<Option<[u8; 32]>>,
    /// The unlocked identity's tamper-evident audit chain
    /// (`rekindle_node::host::audit`). `None` while locked.
    pub audit_chain: Arc<rekindle_node::host::audit::ChainSlot>,
    /// Phase 4 — shared keystore handle so background services that
    /// only have `&AppState` can reach the vault.
    pub keystore: rekindle_vault::VaultHandle,
    /// The one lifecycle FSM (plan E1.1), the Host's (`HostStores`): the
    /// Host makes every identity transition on it.
    pub lifecycle: std::sync::Arc<rekindle_lifecycle::AppLifecycle>,
    /// Phase 8 — shared idempotency cache for mutating commands
    /// returning `Result<(), String>`.
    pub idempotency: std::sync::Arc<rekindle_idempotency::IdempotencyCache<Result<(), String>>>,
    /// Phase 8 — second cache for commands returning a string id
    /// (e.g. `create_channel`). Generics force one cache per `T`.
    pub idempotency_string:
        std::sync::Arc<rekindle_idempotency::IdempotencyCache<Result<String, String>>>,
    /// Phase 9 — cold-start buffer for Veilid updates that arrive before
    /// the app has the state needed to process them.
    pub cold_start: std::sync::Arc<
        rekindle_transport::subscriptions::ColdStartBuffer<veilid_core::VeilidUpdate>,
    >,
    /// The single queue every webview event goes through.
    pub event_dispatch: std::sync::Arc<crate::event_dispatch::EventDispatch>,
    /// Per-window event channels (`subscribe_events`).
    pub event_router: crate::event_router::WebviewRouter,
    /// The in-process host (plan D3.1), set once it has started.
    pub host: super::host_slot::HostSlot,
    /// The node's own routes (plan C7.9a): allocated at readiness, released
    /// and wanted again at every lock (the Host's teardown), released for
    /// good at exit.
    pub own_routes: RwLock<
        Option<
            Arc<
                rekindle_route::own_routes::OwnRoutes<
                    rekindle_protocol::own_routes::VeilidRouteAllocator,
                >,
            >,
        >,
    >,
    /// The process's wall clock (plan C9.2), shared with the in-process
    /// host: the governance, gossip and MEK adapters read it.
    pub clock: Arc<dyn rekindle_utils::Clock>,
    /// The process's generator for the ids and keys the engines mint (plan
    /// C9.2), shared with the in-process host.
    pub rng: Arc<dyn rekindle_utils::RandomSource>,
    /// The personal sync record's watch lease, kept for the session
    /// (`cross_device_sync::watch`); closed by the pool at logout.
    pub personal_sync_lease: Mutex<Option<rekindle_records::lease::LeaseId>>,
    /// The process's importer of peers' private routes (boot-scoped:
    /// routes are imported before login too).
    pub route_imports: RwLock<Option<Arc<rekindle_protocol::dht::route_imports::RouteImports>>>,
    /// `public_internet_ready`, as the attachment handler last saw it; the
    /// pool's retry layer waits on it.
    pub network_ready: tokio::sync::watch::Sender<bool>,
    /// Live community and channel keys by `(community_id, KeyScope)`.
    /// Read and written only through `state_helpers::meks`; consumers read
    /// keys through the `ChannelKeyProvider` (`state_helpers::key_provider`).
    pub meks: Mutex<HashMap<(String, KeyScope), crate::state_helpers::LiveMek>>,
    /// `JoinHandle` for the Veilid dispatch loop.
    pub dispatch_loop_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// `LinkOpen` reviews this shell asked for: their commit opens the URL
    /// the Host answers (`commands::intent::intent_commit`).
    pub pending_link_opens: Mutex<HashSet<rekindle_types::intent::IntentId>>,
    /// An OS deep link that arrived before unlock (raw input; a newer link
    /// replaces it), submitted to the Host by the unlock and cleared at
    /// logout (`deep_links`, plans E2.1, E3.1 D2).
    pub held_deep_link: Mutex<Option<String>>,
    /// Tauri app handle — set during `.setup()`.
    pub app_handle: RwLock<Option<tauri::AppHandle>>,
    /// Global dedup cache for gossip mesh message deduplication.
    pub dedup_cache: Mutex<DedupCache>,
    /// Call-media sender keys per community voice channel session (plan
    /// C7.20): our own key and the keys participants sent us. Kept across
    /// transport rebuilds, ended with the channel session, cleared on
    /// logout.
    pub voice_sender_keys: rekindle_secrets::media_sender_key::keyring::ChannelSenderKeyStore,
    /// M10.4 — receiver-side per-(community, sender) gossip rate floor.
    pub gossip_rate_limits:
        Mutex<HashMap<(String, String), rekindle_gossip::rate_limit::TokenBucket>>,
    /// Per-community compiled AutoMod cache.
    pub automod_cache: Arc<RwLock<HashMap<String, Arc<AutoModCompiledCache>>>>,
    /// The Lost Cargo chunk caches: the Host's store (`HostStores::files`,
    /// plan E3.1 E3c), opened at unlock.
    pub files: Arc<rekindle_files::FileCacheStore>,
    /// Mutual Aid §14.5 dirty set for per-peer reliability counters.
    pub relay_reliability_dirty: Mutex<HashSet<(String, String)>>,
    /// Push relay (architecture §17.3 Tier 3) wake-notify debounce.
    pub last_wake_notify_secs: Mutex<u64>,
    /// Architecture §10.6 video / screen-share reassembly.
    pub video_reassembly: rekindle_video::VideoReassemblyState,
    /// W11.4 — per-peer DM video reassembly state.
    pub dm_video_reassembly: crate::services::dm::video::DmVideoReassemblyState,
    /// Architecture §17.2 line 2402 — per-channel notification burst throttle.
    pub notification_throttle: crate::services::community::notifications::NotificationThrottle,
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
    pub group_calls: Arc<Mutex<HashMap<String, rekindle_calls::group_state::GroupCallState>>>,
    /// Phase 11 Tier 1 — frontend-registered `ipc::Channel` handles for
    /// the high-throughput video streams (DM + community). Frames bypass
    /// the `event_dispatch` bus and go straight to these channels.
    pub video_channels: crate::video_channels::VideoChannelRegistry,
    /// Architecture §10.6 Phase B — per-(community, channel) aggregated
    /// MediaCapabilities + last-emitted `SessionVideoConfig` snapshot.
    /// Recomputed (and idempotently re-emitted) whenever the room
    /// composition changes or a peer reports new caps. Backend owns the
    /// negotiation; the frontend just reads the resulting event.
    pub video_sessions: crate::services::community::video_session::VideoSessionStateMap,
    /// Media-ready session gate (WebRTC "transport before RTP" analog):
    /// per-(community, channel) bring-up inputs → derived ready state.
    /// `media_ready_runtime` owns transitions + the
    /// `CommunityEvent::VoiceMediaReady` emission.
    pub media_ready: Mutex<rekindle_voice::media_ready::MediaReadyTracker>,
    /// Video frames rejected by the media-ready gate (observability).
    pub video_pre_ready_drops: std::sync::atomic::AtomicU64,
    /// Inbound `'D'` and `'G'` datagram queue — the app_message callback
    /// pushes and returns so video processing can never stall voice
    /// packet delivery; one worker (spawned by the dispatch loop)
    /// drains it in FIFO order.
    pub gossip_ingress: Arc<crate::services::veilid::ingress_queue::GossipIngressQueue>,
    /// Piece 6 — inbound control-event queue. The dispatch loop pushes
    /// `ValueChange`/`AppCall` here and returns immediately; a sibling
    /// worker (spawned by the dispatch loop) drains it in FIFO order via
    /// `handle_veilid_update`. Decouples the SLOW identity-dependent
    /// handlers (DHT/DB/decrypt, network round-trips) from the serial
    /// recv loop so incoming media `AppMessage`s can't back up in the
    /// node's 4096-deep update channel behind them.
    pub control_ingress: Arc<crate::services::veilid::control_ingress::ControlIngressQueue>,
    /// Inbound `app_call`s, served concurrently within Veilid's answer
    /// deadline (plan C7.22).
    pub app_call_lane: crate::services::veilid::app_call_lane::AppCallLane,
    /// Linux-native camera session slot (unit type off-Linux).
    pub native_video: crate::services::native_video::NativeVideoSlot,
    /// Phase 5 — last-known halves of the merged ConnectionQuality
    /// emission (send-side quality + receive-side jitter drops arrive
    /// on different 5 s cadences; each emission carries both).
    pub voice_quality_cache:
        Mutex<crate::services::voice_adapter::event_mapping::VoiceQualityCache>,
    /// Phase 5 — CUMULATIVE inbound voice drops (`voice_pkt_drops` is
    /// swap-reset every second by the telemetry poller; this total
    /// feeds the quality event's `ingress_drops`).
    pub voice_ingress_drops_total: std::sync::atomic::AtomicU64,
    /// Channel-media sends that found an empty roster (observability —
    /// frames encoded but with nobody to send to).
    pub channel_send_empty_roster_drops: std::sync::atomic::AtomicU64,
    /// Single-flight gate for per-(community, peer) DHT route
    /// re-resolution. Lives on AppState because the gossip adapter is
    /// rebuilt per send — the coalescing window must span sends.
    pub gossip_resolve_gate: rekindle_gossip::ResolveGate,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            voice_packet_tx: Arc::new(RwLock::new(None)),
            voice_report_tx: Arc::new(RwLock::new(None)),
            identity: Arc::new(RwLock::new(None)),
            communities: rekindle_node::host::community::new_map(),
            node: Arc::new(RwLock::new(None)),
            transport: Arc::new(RwLock::new(None)),
            transport_session: Arc::new(parking_lot::RwLock::new(None)),
            db: Arc::new(rekindle_db::DbHandle::default()),
            identity_attempts: Arc::new(rekindle_idempotency::attempts::AttemptLimiter::default()),
            preferences: RwLock::new(rekindle_types::preferences::Preferences::default()),
            voice_engine: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            identity_secret: Mutex::new(None),
            audit_chain: Arc::new(Mutex::new(None)),
            keystore: rekindle_vault::new_handle(),
            lifecycle: std::sync::Arc::new(rekindle_lifecycle::AppLifecycle::new()),
            idempotency: std::sync::Arc::new(
                rekindle_idempotency::IdempotencyCache::with_defaults(),
            ),
            idempotency_string: std::sync::Arc::new(
                rekindle_idempotency::IdempotencyCache::with_defaults(),
            ),
            cold_start: std::sync::Arc::new(
                rekindle_transport::subscriptions::ColdStartBuffer::new(),
            ),
            event_dispatch: std::sync::Arc::new(crate::event_dispatch::EventDispatch::new()),
            event_router: crate::event_router::WebviewRouter::default(),
            host: super::host_slot::HostSlot::default(),
            own_routes: RwLock::new(None),
            clock: Arc::new(rekindle_utils::SystemClock),
            rng: Arc::new(rekindle_utils::OsRandom),
            personal_sync_lease: Mutex::new(None),
            route_imports: RwLock::new(None),
            network_ready: tokio::sync::watch::Sender::new(false),
            meks: Mutex::new(HashMap::new()),
            dispatch_loop_handle: RwLock::new(None),
            pending_link_opens: Mutex::new(HashSet::new()),
            held_deep_link: Mutex::new(None),
            app_handle: RwLock::new(None),
            dedup_cache: Mutex::new(DedupCache::new(1024)),
            voice_sender_keys:
                rekindle_secrets::media_sender_key::keyring::ChannelSenderKeyStore::default(),
            gossip_rate_limits: Mutex::new(HashMap::new()),
            automod_cache: Arc::new(RwLock::new(HashMap::new())),
            files: Arc::default(),
            relay_reliability_dirty: Mutex::new(HashSet::new()),
            last_wake_notify_secs: Mutex::new(0),
            video_reassembly: rekindle_video::VideoReassemblyState::new(),
            dm_video_reassembly: crate::services::dm::video::DmVideoReassemblyState::new(),
            notification_throttle:
                crate::services::community::notifications::NotificationThrottle::new(),
            active_calls: Arc::new(crate::services::calls_adapter::ActiveCallRegistry::new(
                Arc::new(Mutex::new(HashMap::new())),
            )),
            temp_call_muted: Arc::new(Mutex::new(HashMap::new())),
            voice_pkt_drops: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            voice_packet_rx_staged: parking_lot::Mutex::new(None),
            group_calls: Arc::new(Mutex::new(HashMap::new())),
            video_channels: crate::video_channels::VideoChannelRegistry::new(),
            video_sessions: crate::services::community::video_session::VideoSessionStateMap::new(),
            media_ready: Mutex::new(rekindle_voice::media_ready::MediaReadyTracker::new()),
            gossip_ingress: Arc::new(
                crate::services::veilid::ingress_queue::GossipIngressQueue::new(),
            ),
            app_call_lane: crate::services::veilid::app_call_lane::AppCallLane::default(),
            control_ingress: Arc::new(
                crate::services::veilid::control_ingress::ControlIngressQueue::new(),
            ),
            native_video: crate::services::native_video::NativeVideoSlot::default(),
            voice_quality_cache: Mutex::new(
                crate::services::voice_adapter::event_mapping::VoiceQualityCache::default(),
            ),
            voice_ingress_drops_total: std::sync::atomic::AtomicU64::new(0),
            video_pre_ready_drops: std::sync::atomic::AtomicU64::new(0),
            channel_send_empty_roster_drops: std::sync::atomic::AtomicU64::new(0),
            gossip_resolve_gate: rekindle_gossip::ResolveGate::new(),
        }
    }
}

impl AppState {
    /// The identity slots the in-process host shares (plan D3.1): the host
    /// and every command reach one identity through these `Arc`s.
    #[must_use]
    pub fn host_stores(&self) -> rekindle_node::host::HostStores {
        rekindle_node::host::HostStores {
            db: Arc::clone(&self.db),
            vault: self.keystore.clone(),
            attempts: Arc::clone(&self.identity_attempts),
            audit_chain: Arc::clone(&self.audit_chain),
            session: Arc::clone(&self.transport_session),
            lifecycle: Arc::clone(&self.lifecycle),
            communities: Arc::clone(&self.communities),
            files: Arc::clone(&self.files),
        }
    }

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
}
