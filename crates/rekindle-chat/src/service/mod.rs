//! ChatService — the single entry point for all platform operations.
//!
//! Constructed once per daemon lifetime. Holds `Arc<PlatformIO>` which
//! every service shares, and `Arc<EventPipeline>` which all inbound
//! and local events flow through before reaching IPC clients.
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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use parking_lot::RwLock;
use zeroize::Zeroizing;
use rekindle_types::transport::Transport;
use rekindle_types::subscription_events::SubscriptionEvent;
use rekindle_storage::VaultStore;
use rekindle_types::session_types::SessionMeta;

use crate::crypto::sessions::SessionCache;
use crate::crypto::mek::MekCache;
use crate::events::registry::WatchRegistry;
use crate::events::router::EventRouter;
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
    pub(crate) event_router: Arc<EventRouter>,
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

        // Spawn the inbox scan coordinator first — FriendshipService needs
        // a clone of its trigger sender so the event router can trigger
        // scans without holding a reference to the coordinator.
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

        let messaging = Arc::new(MessagingService {
            io: Arc::clone(&io),
            vault: Arc::clone(&vault),
            session_meta: Arc::clone(&session_meta),
            session_cache: Arc::clone(&session_cache),
            mek_cache: Arc::clone(&mek_cache),
            pipeline: Arc::clone(&pipeline),
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

        let event_router = Arc::new(EventRouter::new(
            Arc::clone(&watches),
            Arc::clone(&pipeline),
            Arc::clone(&friendship),
            Arc::clone(&messaging),
            Arc::clone(&community),
        ));

        Ok(Self {
            io, vault, session_meta, session_cache, mek_cache,
            watches, pipeline, event_router,
            friendship, messaging, community, identity, presence, voice,
            inbox_scan: Some(inbox_scan),
            session_path,
            session_mac_key: Zeroizing::new(session_mac_key),
            session_dirty: AtomicBool::new(false),
        })
    }

    pub fn callback(&self) -> Arc<dyn rekindle_types::transport::TransportCallback> {
        Arc::clone(&self.event_router) as Arc<dyn rekindle_types::transport::TransportCallback>
    }

    pub fn io(&self) -> &Arc<PlatformIO> {
        &self.io
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SubscriptionEvent> {
        self.pipeline.subscribe()
    }

    /// Emit a locally-originated event through the pipeline.
    pub fn emit_local(&self, event: SubscriptionEvent) {
        self.pipeline.process(event);
    }

    /// Whether the platform is operational (signing key loaded + transport attached).
    pub fn is_operational(&self) -> bool {
        self.io.is_signing_key_loaded() && self.io.is_attached()
    }

    /// Access the pipeline's broadcast sender for IPC event wiring.
    pub fn pipeline_sender(&self) -> &tokio::sync::broadcast::Sender<SubscriptionEvent> {
        self.pipeline.sender()
    }

    /// Trigger an inbox scan (non-blocking). Coalesced by 30s cooldown.
    pub fn trigger_inbox_scan(&self) {
        if let Some(ref scan) = self.inbox_scan {
            scan.trigger();
        }
    }
}
