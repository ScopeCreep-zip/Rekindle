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

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use tokio::sync::mpsc;

// Re-export from rekindle-types — the single source of truth for IPC-boundary types.
pub use rekindle_types::notification::{AttachmentState, TransportNotification};

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
    /// Reliable peers in the routing table (0.5.7 attachment signal).
    reliable_peer_count: AtomicU64,
    /// Live peers — reliable, unreliable, and newly added (0.5.7).
    live_peer_count: AtomicU64,
    /// Smoothed estimate of total reachable network size (0.5.7).
    estimated_network_size: AtomicU64,
    /// Median p75 latency across reliable peers, in microseconds
    /// (0 = no samples yet).
    median_latency_us: AtomicU64,
    /// Dead-route heal attempts admitted by the `HealGate` (A8 telemetry —
    /// baseline for retuning the route-heal timers post-0.5.7).
    heal_attempts_admitted: AtomicU64,
    /// Dead-route heal attempts suppressed by the cooldown.
    heal_attempts_suppressed: AtomicU64,
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
            reliable_peer_count: AtomicU64::new(0),
            live_peer_count: AtomicU64::new(0),
            estimated_network_size: AtomicU64::new(0),
            median_latency_us: AtomicU64::new(0),
            heal_attempts_admitted: AtomicU64::new(0),
            heal_attempts_suppressed: AtomicU64::new(0),
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

    /// Record the richer 0.5.7 attachment health signal (peer counts,
    /// network-size estimate, median latency). Called by the dispatch
    /// loop alongside [`set_attachment`](Self::set_attachment); readiness
    /// checks and status surfaces read these instead of inferring health
    /// from the 8-value attachment enum alone.
    pub fn set_network_health(
        &self,
        reliable_peers: u64,
        live_peers: u64,
        estimated_network_size: u64,
        median_latency_us: u64,
    ) {
        self.reliable_peer_count
            .store(reliable_peers, Ordering::Release);
        self.live_peer_count.store(live_peers, Ordering::Release);
        self.estimated_network_size
            .store(estimated_network_size, Ordering::Release);
        self.median_latency_us
            .store(median_latency_us, Ordering::Release);
    }

    /// Count one dead-route heal decision (A8 telemetry). `admitted` is
    /// the `HealGate::try_begin` result: `true` = heal ran, `false` =
    /// suppressed by the cooldown. The admitted/suppressed ratio is the
    /// baseline the route-heal timers get retuned against post-0.5.7.
    pub fn count_heal_attempt(&self, admitted: bool) {
        if admitted {
            self.heal_attempts_admitted.fetch_add(1, Ordering::Relaxed);
        } else {
            self.heal_attempts_suppressed
                .fetch_add(1, Ordering::Relaxed);
        }
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

    /// Reliable peers in the routing table (lock-free atomic read).
    pub fn reliable_peer_count(&self) -> u64 {
        self.reliable_peer_count.load(Ordering::Acquire)
    }

    /// Live peers (reliable + unreliable + newly added) in the routing table.
    pub fn live_peer_count(&self) -> u64 {
        self.live_peer_count.load(Ordering::Acquire)
    }

    /// Smoothed estimate of total reachable network size.
    pub fn estimated_network_size(&self) -> u64 {
        self.estimated_network_size.load(Ordering::Acquire)
    }

    /// Median p75 latency across reliable peers in microseconds
    /// (0 = no samples yet).
    pub fn median_latency_us(&self) -> u64 {
        self.median_latency_us.load(Ordering::Acquire)
    }

    /// Heal-gate decisions so far: `(admitted, suppressed)` (A8 telemetry).
    pub fn heal_attempt_counts(&self) -> (u64, u64) {
        (
            self.heal_attempts_admitted.load(Ordering::Relaxed),
            self.heal_attempts_suppressed.load(Ordering::Relaxed),
        )
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

/// Transport-layer status snapshot — SSOT definition in `rekindle-types`.
pub use rekindle_types::display::TransportSnapshot;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_state_round_trip() {
        for raw in 0..=8u8 {
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

    /// Every attached state veilid-core can actually emit must map to an
    /// attached state on our side.
    ///
    /// `from_veilid_string` fails closed to `Detached`, so an upstream
    /// rename or a new variant does not break the build — it silently
    /// reports a healthy node as detached. veilid-core 0.5.4 did both at
    /// once (added `attached_fair`, renamed `FullyAttached` ->
    /// `AttachedFull`) and the changelog mentioned neither. This walks
    /// upstream's own enum so the next one fails here instead.
    #[test]
    fn every_veilid_attachment_string_is_understood() {
        use veilid_core::AttachmentState as Upstream;

        for upstream in [
            Upstream::Detached,
            Upstream::Attaching,
            Upstream::AttachedWeak,
            Upstream::AttachedFair,
            Upstream::AttachedGood,
            Upstream::AttachedStrong,
            Upstream::AttachedFull,
            Upstream::Detaching,
        ] {
            let rendered = upstream.to_string();
            let ours = AttachmentState::from_veilid_string(&rendered);

            assert_eq!(
                ours.is_attached(),
                upstream.is_attached(),
                "{rendered:?} parsed to {ours:?}, whose is_attached() \
                 disagrees with veilid-core"
            );

            if !matches!(upstream, Upstream::Detached) {
                assert_ne!(
                    ours,
                    AttachmentState::Detached,
                    "{rendered:?} fell through to the fail-closed Detached \
                     arm — add it to from_veilid_string"
                );
            }
        }
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
            TransportNotification::AttachmentChanged {
                state,
                is_attached,
                public_internet_ready,
            } => {
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
