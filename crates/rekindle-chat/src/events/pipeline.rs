//! EventPipeline — the single path for ALL event emission.
//!
//! Every SubscriptionEvent in the system — whether from transport (inbound
//! gossip, watch, poll) or from local actions (user sends a message) —
//! flows through this pipeline before reaching any client.
//!
//! Pipeline steps:
//! 1. `state_effects::apply` — update reactive state (unread, typing, presence, voice)
//! 2. Emit extra events from state_effects (e.g., UnreadChanged after unread increment)
//! 3. `EventDedup::check` — suppress duplicates from parallel delivery tiers
//! 4. `event_tx.send` — broadcast to all IPC subscribers
//!
//! Two entry points:
//! - `EventRouter::on_message` for transport-originated events
//! - `ChatService::emit_local` for locally-originated events
//!
//! Both call `pipeline.process(event)`. Same path. Same dedup. Same
//! state effects. No event bypasses this pipeline.

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::broadcast;

use rekindle_types::subscription_events::SubscriptionEvent;

use super::dedup::EventDedup;
use super::state::SubscriptionState;
use super::state_effects;

/// Broadcast channel capacity for subscription events.
const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// The sole event emission path for the entire platform.
///
/// Shared by `EventRouter` (inbound) and `ChatService` (local).
/// Holds the reactive state, dedup cache, and broadcast channel.
pub struct EventPipeline {
    dedup: Arc<RwLock<EventDedup>>,
    state: Arc<RwLock<SubscriptionState>>,
    event_tx: broadcast::Sender<SubscriptionEvent>,
}

impl EventPipeline {
    pub fn new(
        dedup: Arc<RwLock<EventDedup>>,
        state: Arc<RwLock<SubscriptionState>>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { dedup, state, event_tx }
    }

    /// Process an event through the full pipeline:
    /// state_effects → dedup → emit.
    ///
    /// This is the ONLY way events reach the IPC bus. No exceptions.
    pub fn process(&self, event: SubscriptionEvent) {
        // Step 1: Apply state side-effects (unread, typing, presence, voice).
        // Returns additional events to emit (e.g., UnreadChanged).
        let extra_events = {
            let mut state = self.state.write();
            state_effects::apply(&mut state, &event)
        };
        // State lock released here before dedup + send.

        // Step 2: Dedup — suppress if this exact event was already emitted.
        // Dedup operates on semantic content hash, not identity — the same
        // logical event arriving via watch, gossip, AND poll is suppressed
        // to a single emission.
        {
            let mut dedup = self.dedup.write();
            if dedup.check(&event) {
                // New event — emit to all subscribers.
                if let Err(e) = self.event_tx.send(event) {
                    tracing::trace!(
                        "event emission: no active subscribers ({:?})",
                        e.0
                    );
                }
            }
            // Duplicate — silently suppressed. This is correct behavior,
            // not an error. The same event was already delivered via a
            // faster tier.
        }
        // Dedup lock released here.

        // Step 3: Emit extra events (e.g., UnreadChanged).
        // These are ALWAYS emitted — they are computed aggregates,
        // not network events, so dedup does not apply (EventDedup
        // special-cases UnreadChanged to always pass).
        for extra in extra_events {
            if let Err(e) = self.event_tx.send(extra) {
                tracing::trace!(
                    "extra event emission: no active subscribers ({:?})",
                    e.0
                );
            }
        }
    }

    /// Subscribe to all events. Returns a broadcast receiver.
    /// Multiple subscribers supported. Dropping the receiver auto-unsubscribes.
    pub fn subscribe(&self) -> broadcast::Receiver<SubscriptionEvent> {
        self.event_tx.subscribe()
    }

    /// Access the broadcast sender for direct wiring to IPC.
    pub fn sender(&self) -> &broadcast::Sender<SubscriptionEvent> {
        &self.event_tx
    }

    /// Access the reactive state for client queries (unread, typing, presence, voice).
    pub fn state(&self) -> &Arc<RwLock<SubscriptionState>> {
        &self.state
    }

    /// Access the dedup cache for diagnostics.
    pub fn dedup(&self) -> &Arc<RwLock<EventDedup>> {
        &self.dedup
    }

    /// Evict expired dedup entries. Called periodically by a background task.
    pub fn evict_expired_dedup(&self) {
        self.dedup.write().evict_expired();
    }
}
