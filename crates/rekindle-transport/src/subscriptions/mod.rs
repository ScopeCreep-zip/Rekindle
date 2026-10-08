//! Consolidated subscription module — every inbound signal from Veilid.
//!
//! `SubscriptionManager` is the **sole owner** of all reactive state:
//! DHT watches, gossip processing, typing/presence/unread tracking,
//! and event emission. No other code in the workspace establishes
//! watches, processes ValueChange events, or maintains unread counts.
//!
//! # Submodules
//!
//! - `events/` — typed event enums per domain (channel, typing, presence, etc.)
//! - `state` — mutable state: unread, typing, presence, voice
//! - `watches` — DHT watch lifecycle: create, renew, route ValueChange
//! - `gossip_handler` — inbound gossip processing (all 52 ControlPayload variants)
//! - `dm_handler` — inbound DM processing (all 10 DmPayload variants)

pub mod cold_start;
pub mod events;
// `state`, `state_effects`, and `dedup` were hoisted to `rekindle-events` in
// Phase 1 of the decomposed-harvest plan. They are re-exported below so
// internal callers (and historical `subscriptions::state::SubscriptionState`
// paths from elsewhere in the workspace) keep working unchanged.
pub use rekindle_events::{dedup, state, state_effects};
mod bare_envelope;
pub mod dispatch;
mod dispatch_gossip;
mod manager_ingress;
mod manager_lifecycle;
mod manager_network;
mod manager_setup;
pub mod poll;
mod read_state;
pub mod watches;

pub use cold_start::ColdStartBuffer;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_lifecycle::SessionScope;
use tokio::sync::broadcast;

use crate::broadcast::node::TransportNode;
use crate::gossip::GossipMesh;
use crate::session::Session;

use events::SubscriptionEvent;
use state::SubscriptionState;
use watches::WatchRegistry;

/// Broadcast channel capacity for subscription events.
const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// Centralized subscription manager.
///
/// Owns all inbound reactive state. Created once during daemon resume.
/// The daemon's `InboundHandler` forwards every signal here. Consumers
/// (TUI, CLI) subscribe via `subscribe()` and receive typed events.
pub struct SubscriptionManager {
    /// Transport node for DHT operations.
    node: Arc<TransportNode>,
    /// Session state (identity, communities, DM keys).
    session: Arc<RwLock<Option<Session>>>,
    /// Mutable state: unread counts, typing, presence, voice.
    state: Arc<RwLock<SubscriptionState>>,
    /// Active DHT watches registry.
    watches: Arc<RwLock<WatchRegistry>>,
    /// Cross-tier Blake3 content deduplication.
    dedup: Arc<RwLock<dedup::EventDedup>>,
    /// Per-community gossip mesh (shared with BroadcastManager).
    meshes: Arc<RwLock<HashMap<String, GossipMesh>>>,
    /// Broadcast sender for subscription events.
    event_tx: broadcast::Sender<SubscriptionEvent>,
    /// The scope its background tasks (renewal, poll, maintenance,
    /// watch re-establishment) run in; a child of the host's unlock
    /// scope, so locking stops them (plan C4).
    scope: Arc<SessionScope>,
}

impl SubscriptionManager {
    /// Create a new subscription manager. Does NOT start background tasks.
    ///
    /// Call `setup_identity()` and `setup_community()` to begin watching.
    /// Background tasks run in `scope`.
    pub fn new(
        node: Arc<TransportNode>,
        session: Arc<RwLock<Option<Session>>>,
        scope: Arc<SessionScope>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            node,
            session,
            state: Arc::new(RwLock::new(SubscriptionState::default())),
            watches: Arc::new(RwLock::new(WatchRegistry::new())),
            dedup: Arc::new(RwLock::new(dedup::EventDedup::default())),
            meshes: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            scope,
        }
    }

    /// Subscribe to all subscription events. Multiple subscribers supported.
    /// Dropping the receiver auto-unsubscribes.
    pub fn subscribe(&self) -> broadcast::Receiver<SubscriptionEvent> {
        self.event_tx.subscribe()
    }

    /// Access the broadcast sender for passing to the IPC event delivery system.
    ///
    /// The IPC server uses this to subscribe internally and route events
    /// through the EventRouter to connected clients.
    pub fn event_sender(&self) -> &broadcast::Sender<SubscriptionEvent> {
        &self.event_tx
    }
}
