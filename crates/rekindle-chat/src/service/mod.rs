//! ChatService — the single entry point for all platform operations.
//!
//! Constructed once per daemon lifetime. Holds `Arc<PlatformIO>` which
//! every service shares, and `Arc<EventPipeline>` which all inbound
//! and local events flow through before reaching IPC clients.
//!
//! Inbound data arrives via `mpsc::Receiver<InboundEvent>` from the
//! transport layer. Call `start_inbound_loop()` after construction
//! to spawn the reader. No TransportCallback trait. No set_callback().
//!
//! Method groups are split into submodules for maintainability:
//! - `resume.rs` — DHT record reopening, route publishing, watch setup
//! - `state.rs` — read-only state queries (unread, typing, presence, voice)
//! - `delegate.rs` — forwarding methods to domain services
//! - `background.rs` — periodic tasks, session persistence, lock

mod resume;
mod state;
mod delegate;
mod background;
mod dm_runtime;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use std::collections::HashMap;

use parking_lot::RwLock;
use zeroize::Zeroizing;
use rekindle_types::transport::{Transport, InboundEvent};
use rekindle_types::subscription_events::SubscriptionEvent;
use rekindle_storage::VaultStore;
use rekindle_types::session_types::SessionMeta;

use crate::crypto::sessions::SessionCache;
use crate::crypto::mek::MekCache;
use crate::events::registry::WatchRegistry;
use crate::events::pipeline::EventPipeline;
use crate::events::dedup::EventDedup;
use crate::events::state::SubscriptionState;
use crate::friendship::FriendshipService;
use crate::friendship::inbox::InboxScanCoordinator;
use crate::messaging::MessagingService;
use crate::community::CommunityService;
use crate::identity::IdentityService;
use crate::presence::PresenceService;
use crate::voice::session::VoiceService;
use crate::io::PlatformIO;
use crate::ChatError;

pub struct ChatService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) vault: Arc<VaultStore>,
    pub(crate) session_meta: Arc<RwLock<SessionMeta>>,
    pub(crate) session_cache: Arc<SessionCache>,
    pub(crate) mek_cache: Arc<MekCache>,
    pub(crate) watches: Arc<WatchRegistry>,
    pub(crate) pipeline: Arc<EventPipeline>,
    pub(crate) friendship: Arc<FriendshipService>,
    pub(crate) messaging: Arc<MessagingService>,
    pub(crate) community: Arc<CommunityService>,
    pub(crate) identity: IdentityService,
    pub(crate) presence: PresenceService,
    pub(crate) voice: VoiceService,
    pub(crate) inbox_scan: Option<InboxScanCoordinator>,
    session_path: PathBuf,
    session_mac_key: Zeroizing<[u8; 32]>,
    session_dirty: AtomicBool,
    pub(crate) dm_deps: Arc<dyn crate::dm::DmDeps>,
}

impl ChatService {
    pub fn new(
        transport: Arc<dyn Transport>,
        vault: Arc<VaultStore>,
        session_meta: SessionMeta,
        session_path: PathBuf,
        session_mac_key: [u8; 32],
    ) -> Result<Self, ChatError> {
        let io = Arc::new(PlatformIO::new(transport));

        let mut meta = session_meta;
        let names = vault.load_friend_names()?;
        for (k, v) in names {
            meta.friend_display_names.insert(k, v);
        }
        let session_meta = Arc::new(RwLock::new(meta));

        let session_cache = Arc::new(SessionCache::new(Arc::clone(&vault)));
        let mek_cache = Arc::new(MekCache::from_vault(Arc::clone(&vault))?);
        let watches = Arc::new(WatchRegistry::new());
        let dedup = Arc::new(RwLock::new(EventDedup::default()));
        let state = Arc::new(RwLock::new(SubscriptionState::default()));
        let pipeline = Arc::new(EventPipeline::new(dedup, state));

        let inbox_scan = InboxScanCoordinator::spawn(
            Arc::clone(&io),
            Arc::clone(&vault),
            Arc::clone(&session_meta),
            Arc::clone(&session_cache),
            Arc::clone(&watches),
            Arc::clone(&pipeline),
        );

        let friendship = Arc::new(FriendshipService {
            io: Arc::clone(&io),
            vault: Arc::clone(&vault),
            session_meta: Arc::clone(&session_meta),
            session_cache: Arc::clone(&session_cache),
            watches: Arc::clone(&watches),
            inbox_trigger: inbox_scan.trigger_sender(),
        });

        let (retry_tx, retry_rx) = tokio::sync::mpsc::channel(256);
        let messaging = Arc::new(MessagingService {
            io: Arc::clone(&io),
            vault: Arc::clone(&vault),
            session_meta: Arc::clone(&session_meta),
            session_cache: Arc::clone(&session_cache),
            mek_cache: Arc::clone(&mek_cache),
            pipeline: Arc::clone(&pipeline),
            slowmode_last_send: parking_lot::Mutex::new(std::collections::HashMap::new()),
            retry_tx,
            retry_rx: parking_lot::Mutex::new(Some(retry_rx)),
        });

        let community = Arc::new(CommunityService {
            io: Arc::clone(&io),
            vault: Arc::clone(&vault),
            session_meta: Arc::clone(&session_meta),
            mek_cache: Arc::clone(&mek_cache),
            watches: Arc::clone(&watches),
            pipeline: Arc::clone(&pipeline),
        });

        let identity = IdentityService {
            io: Arc::clone(&io),
            vault: Arc::clone(&vault),
            session_meta: Arc::clone(&session_meta),
        };

        let presence = PresenceService {
            io: Arc::clone(&io),
            session_meta: Arc::clone(&session_meta),
        };

        let voice = VoiceService::new(
            Arc::clone(&io),
            Arc::clone(&mek_cache),
        );

        let dm_deps: Arc<dyn crate::dm::DmDeps> = Arc::new(dm_runtime::ChatDmRuntime {
            io: Arc::clone(&io),
            vault: Arc::clone(&vault),
            session_cache: Arc::clone(&session_cache),
            session_meta: Arc::clone(&session_meta),
            pipeline: Arc::clone(&pipeline),
            mek_chains: RwLock::new(HashMap::new()),
        });

        Ok(Self {
            io, vault, session_meta, session_cache, mek_cache,
            watches, pipeline,
            friendship, messaging, community, identity, presence, voice, dm_deps,
            inbox_scan: Some(inbox_scan),
            session_path,
            session_mac_key: Zeroizing::new(session_mac_key),
            session_dirty: AtomicBool::new(false),
        })
    }

    /// Spawn the inbound event reader loop. Reads InboundEvent from the
    /// transport's mpsc channel and dispatches to services.
    ///
    /// Replaces the old `transport.set_callback(Arc::new(EventRouter))` pattern.
    /// No trait object, no lazy installation, no RwLock.
    pub fn start_inbound_loop(&self, rx: tokio::sync::mpsc::Receiver<InboundEvent>) {
        let watches = Arc::clone(&self.watches);
        let pipeline = Arc::clone(&self.pipeline);
        let friendship = Arc::clone(&self.friendship);
        let messaging = Arc::clone(&self.messaging);
        let community = Arc::clone(&self.community);
        let dm_deps = Arc::clone(&self.dm_deps);
        tokio::spawn(crate::events::router::run_inbound_loop(
            rx, watches, pipeline, friendship, messaging, community, dm_deps,
        ));
    }

    pub fn io(&self) -> &Arc<PlatformIO> { &self.io }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SubscriptionEvent> {
        self.pipeline.subscribe()
    }

    pub fn emit_local(&self, event: SubscriptionEvent) {
        self.pipeline.process(event);
    }

    pub fn is_operational(&self) -> bool {
        self.io.is_identity_loaded() && self.io.is_attached()
    }

    pub fn pipeline_sender(&self) -> &tokio::sync::broadcast::Sender<SubscriptionEvent> {
        self.pipeline.sender()
    }

    pub fn trigger_inbox_scan(&self) {
        if let Some(ref scan) = self.inbox_scan {
            scan.trigger();
        }
    }
}
