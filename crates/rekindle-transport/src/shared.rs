//! Observable shared state for the transport node.
//!
//! [`SharedState`] provides lock-free reads of node attachment state and
//! a broadcast channel for transport notifications. Updated by the dispatch
//! loop, read by any number of CLI/TUI consumers.
//!
//! # Thread Safety
//!
//! Attachment state uses atomics for lock-free reads. The subscriber list
//! uses a `parking_lot::RwLock` which is only write-locked when adding or
//! cleaning up subscribers — never in the hot path of reading state.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::mpsc;

// ── Attachment state ────────────────────────────────────────────────────

/// Network attachment state. Maps from Veilid's string representation.
///
/// Ordered by "goodness" — higher discriminant values indicate stronger
/// attachment. This is a stable ABI contract consumed by CLI display code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[repr(u8)]
pub enum AttachmentState {
    Detached = 0,
    Attaching = 1,
    AttachedWeak = 2,
    AttachedGood = 3,
    AttachedStrong = 4,
    FullyAttached = 5,
    OverAttached = 6,
    Detaching = 7,
}

impl AttachmentState {
    /// Parse from Veilid's attachment state string representation.
    ///
    /// Unknown strings map to `Detached` (fail closed — if we can't parse
    /// the state, we assume the worst).
    pub fn from_veilid_string(s: &str) -> Self {
        match s {
            "Detached" => Self::Detached,
            "Attaching" => Self::Attaching,
            "AttachedWeak" => Self::AttachedWeak,
            "AttachedGood" => Self::AttachedGood,
            "AttachedStrong" => Self::AttachedStrong,
            "FullyAttached" => Self::FullyAttached,
            "OverAttached" => Self::OverAttached,
            "Detaching" => Self::Detaching,
            unknown => {
                tracing::warn!(state = unknown, "unknown Veilid attachment state, treating as Detached");
                Self::Detached
            }
        }
    }

    /// Whether this state represents an attached (usable) network.
    pub fn is_attached(self) -> bool {
        matches!(
            self,
            Self::AttachedWeak
                | Self::AttachedGood
                | Self::AttachedStrong
                | Self::FullyAttached
                | Self::OverAttached
        )
    }

    /// Human-readable label for display.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detached => "Detached",
            Self::Attaching => "Attaching",
            Self::AttachedWeak => "AttachedWeak",
            Self::AttachedGood => "AttachedGood",
            Self::AttachedStrong => "AttachedStrong",
            Self::FullyAttached => "FullyAttached",
            Self::OverAttached => "OverAttached",
            Self::Detaching => "Detaching",
        }
    }

    /// Parse from the atomic u8 storage. Returns `Detached` for out-of-range values.
    fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Attaching,
            2 => Self::AttachedWeak,
            3 => Self::AttachedGood,
            4 => Self::AttachedStrong,
            5 => Self::FullyAttached,
            6 => Self::OverAttached,
            7 => Self::Detaching,
            _ => Self::Detached,
        }
    }
}

impl std::fmt::Display for AttachmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Transport notification ──────────────────────────────────────────────

/// Broadcast notification from the transport layer to CLI/TUI consumers.
///
/// These are summaries of transport events — not the full event payloads.
/// They carry enough context for display decisions (e.g., "show a toast",
/// "refresh the peer list", "update the message list") without exposing
/// transport internals.
///
/// Distinguished from [`crate::handler::TransportEvent`] which is the
/// handler-specific callback type used internally by the dispatch loop.
#[derive(Debug, Clone)]
pub enum TransportNotification {
    /// Network attachment state changed.
    AttachmentChanged {
        state: AttachmentState,
        is_attached: bool,
        public_internet_ready: bool,
    },

    /// One or more local private routes died.
    LocalRoutesDied { count: usize },

    /// One or more remote peer routes died.
    RemoteRoutesDied { peer_keys: Vec<String> },

    /// A DHT watch expired or was cancelled.
    WatchDied { record_key: String },

    /// A DM was received from a verified peer.
    DmReceived {
        sender_key: String,
        sender_name: String,
        timestamp: u64,
    },

    /// A gossip broadcast was received in a community.
    GossipReceived {
        community_id: String,
        sender_pseudonym: String,
        /// Channel ID if this is a channel message notification.
        channel_id: Option<String>,
        /// Message ID if this is a message notification.
        message_id: Option<String>,
        lamport_ts: u64,
    },

    /// A DHT value changed (watch notification).
    ValueChanged {
        record_key: String,
        changed_subkeys: Vec<u32>,
    },

    /// MEK was rotated for a community/channel.
    MekRotated {
        community_id: String,
        channel_id: Option<String>,
        generation: u64,
    },

    /// A voice participant joined.
    VoiceJoin {
        community_id: String,
        channel_id: String,
        participant_key: String,
    },

    /// A voice participant left.
    VoiceLeave {
        community_id: String,
        channel_id: String,
        participant_key: String,
    },
}

// ── Shared state ────────────────────────────────────────────────────────

/// Observable shared state for the transport node.
///
/// Created once at `TransportNode::start()`, shared via `Arc` between the
/// dispatch loop (writer) and any number of CLI/TUI consumers (readers).
///
/// Attachment state reads are lock-free via atomics. Subscriber management
/// uses a `RwLock` that is write-locked only on subscribe/cleanup — the
/// `notify()` hot path holds a read lock to iterate existing subscribers,
/// upgrading to write only when dead subscribers need removal.
pub struct SharedState {
    /// Current attachment state, stored as u8 discriminant.
    attachment: AtomicU8,
    /// Whether the node is attached to the network.
    is_attached: AtomicBool,
    /// Whether the public internet is reachable.
    public_internet_ready: AtomicBool,
    /// Timestamp when the node was started.
    started_at: Instant,
    /// Broadcast subscribers. Each subscriber gets a clone of every notification.
    /// Dead subscribers (receiver dropped) are cleaned up on the next `notify()`.
    subscribers: RwLock<Vec<mpsc::UnboundedSender<TransportNotification>>>,
}

impl SharedState {
    /// Create a new shared state with initial values.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            attachment: AtomicU8::new(AttachmentState::Detached as u8),
            is_attached: AtomicBool::new(false),
            public_internet_ready: AtomicBool::new(false),
            started_at: Instant::now(),
            subscribers: RwLock::new(Vec::new()),
        })
    }

    // ── Writers (called by dispatch loop) ────────────────────────────

    /// Update attachment state and notify all subscribers.
    ///
    /// Called by the dispatch loop when a `VeilidUpdate::Attachment` arrives.
    pub fn set_attachment(&self, state: AttachmentState, attached: bool, pir: bool) {
        self.attachment.store(state as u8, Ordering::Release);
        self.is_attached.store(attached, Ordering::Release);
        self.public_internet_ready.store(pir, Ordering::Release);

        self.notify(&TransportNotification::AttachmentChanged {
            state,
            is_attached: attached,
            public_internet_ready: pir,
        });
    }

    /// Broadcast a notification to all subscribers.
    ///
    /// Subscribers whose receivers have been dropped are automatically removed.
    /// This is the only place subscribers are cleaned up — no background task
    /// needed.
    pub fn notify(&self, event: &TransportNotification) {
        let mut subs = self.subscribers.write();
        subs.retain(|tx| tx.send(event.clone()).is_ok());
    }

    // ── Readers (called by CLI/TUI, lock-free) ──────────────────────

    /// Current attachment state (lock-free atomic read).
    pub fn attachment_state(&self) -> AttachmentState {
        AttachmentState::from_u8(self.attachment.load(Ordering::Acquire))
    }

    /// Whether the node is currently attached to the network.
    pub fn is_attached(&self) -> bool {
        self.is_attached.load(Ordering::Acquire)
    }

    /// Whether the public internet is reachable via the node.
    pub fn public_internet_ready(&self) -> bool {
        self.public_internet_ready.load(Ordering::Acquire)
    }

    /// Time elapsed since the node was started.
    pub fn uptime(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }

    /// Subscribe to transport notifications. Returns a receiver that gets
    /// a clone of every notification broadcast by the dispatch loop.
    ///
    /// Multiple subscribers are supported. Dropping the receiver automatically
    /// unsubscribes on the next `notify()` call.
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<TransportNotification> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.subscribers.write().push(tx);
        rx
    }

    /// Number of active subscribers (for diagnostics).
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.read().len()
    }
}

impl std::fmt::Debug for SharedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState")
            .field("attachment", &self.attachment_state())
            .field("is_attached", &self.is_attached())
            .field("public_internet_ready", &self.public_internet_ready())
            .field("uptime", &self.uptime())
            .field("subscribers", &self.subscriber_count())
            .finish()
    }
}

// ── Node status snapshot ────────────────────────────────────────────────

/// Point-in-time snapshot of node status for display.
///
/// All fields are display-ready primitives — no Veilid types, no locks
/// needed to read. Suitable for JSON serialization and TUI rendering.
#[derive(Debug, Clone, Serialize)]
pub struct NodeStatusSnapshot {
    /// Current attachment state label.
    pub attachment: String,
    /// Whether the node is attached.
    pub is_attached: bool,
    /// Whether public internet is reachable.
    pub public_internet_ready: bool,
    /// Uptime in seconds since node start.
    pub uptime_secs: u64,
    /// Number of known peers with valid (non-stale) routes.
    pub peer_count: usize,
    /// Whether a private route is currently allocated.
    pub route_allocated: bool,
    /// Age of the current route allocation in seconds, if any.
    pub route_age_secs: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_state_round_trip() {
        for raw in 0..=7u8 {
            let state = AttachmentState::from_u8(raw);
            assert_eq!(state as u8, raw);
        }
        // Out of range falls back to Detached
        assert_eq!(AttachmentState::from_u8(255), AttachmentState::Detached);
    }

    #[test]
    fn attachment_state_from_veilid_string() {
        assert_eq!(
            AttachmentState::from_veilid_string("FullyAttached"),
            AttachmentState::FullyAttached
        );
        assert_eq!(
            AttachmentState::from_veilid_string("garbage"),
            AttachmentState::Detached
        );
    }

    #[test]
    fn is_attached_correct() {
        assert!(!AttachmentState::Detached.is_attached());
        assert!(!AttachmentState::Attaching.is_attached());
        assert!(AttachmentState::AttachedWeak.is_attached());
        assert!(AttachmentState::AttachedGood.is_attached());
        assert!(AttachmentState::FullyAttached.is_attached());
        assert!(!AttachmentState::Detaching.is_attached());
    }

    #[tokio::test]
    async fn shared_state_subscribe_receives_events() {
        let state = SharedState::new();
        let mut rx = state.subscribe();

        state.set_attachment(AttachmentState::FullyAttached, true, true);

        let event = rx.try_recv().expect("should receive notification");
        match event {
            TransportNotification::AttachmentChanged { state, is_attached, public_internet_ready } => {
                assert_eq!(state, AttachmentState::FullyAttached);
                assert!(is_attached);
                assert!(public_internet_ready);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn shared_state_cleans_up_dropped_subscribers() {
        let state = SharedState::new();
        let rx1 = state.subscribe();
        let _rx2 = state.subscribe();
        assert_eq!(state.subscriber_count(), 2);

        // Drop rx1
        drop(rx1);

        // Next notify cleans up the dead subscriber
        state.notify(&TransportNotification::LocalRoutesDied { count: 1 });
        assert_eq!(state.subscriber_count(), 1);
    }

    #[test]
    fn shared_state_uptime_increases() {
        let state = SharedState::new();
        let t1 = state.uptime();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = state.uptime();
        assert!(t2 > t1);
    }

    #[test]
    fn shared_state_atomic_reads_match_writes() {
        let state = SharedState::new();
        assert_eq!(state.attachment_state(), AttachmentState::Detached);
        assert!(!state.is_attached());
        assert!(!state.public_internet_ready());

        state.set_attachment(AttachmentState::AttachedStrong, true, true);
        assert_eq!(state.attachment_state(), AttachmentState::AttachedStrong);
        assert!(state.is_attached());
        assert!(state.public_internet_ready());

        state.set_attachment(AttachmentState::Detaching, false, false);
        assert_eq!(state.attachment_state(), AttachmentState::Detaching);
        assert!(!state.is_attached());
        assert!(!state.public_internet_ready());
    }
}
