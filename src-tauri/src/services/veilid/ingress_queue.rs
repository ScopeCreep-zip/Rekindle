//! Inbound dispatch isolation — the voice fast path's bodyguard.
//!
//! The Veilid update dispatch loop is SERIAL: every `app_message` used
//! to await full gossip processing (signature verify → dedup →
//! reassembly → MEK decrypt → IPC emit) inline, so a burst of video
//! fragments stalled voice packet delivery long enough to overflow the
//! bounded voice channel (the Linux dropout). This queue decouples
//! them: the `app_message` callback classifies + pushes and RETURNS;
//! one worker drains in FIFO order (per-sender fragment ordering is
//! load-bearing for reassembly, so exactly one worker).
//!
//! Overflow policy is audio-first by construction — voice packets
//! never enter this queue — and control-first within it: when full,
//! the OLDEST queued VIDEO item is evicted before anything else; a
//! non-video item is only dropped when the queue is somehow all
//! control traffic (pathological; counted + warned).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use rekindle_protocol::dht::community::envelope::SignedEnvelope;
use tokio::sync::Notify;

/// Bounded depth. At the paced video budget (~350 kbps ≈ 4 fragments/s
/// per sender) this is minutes of control traffic or ~10 s of a
/// worst-case unpaced video flood — deep enough to absorb bursts,
/// shallow enough to bound worst-case processing lag.
pub const INGRESS_QUEUE_MAX: usize = 512;

/// One inbound app_message awaiting processing.
pub enum IngressItem {
    /// A decoded community gossip envelope. `is_video` is classified
    /// once at push time (fragment/parity payloads) and drives the
    /// drop-video-first overflow policy.
    Gossip {
        signed: SignedEnvelope,
        is_video: bool,
    },
    /// Everything that isn't voice or gossip — the legacy
    /// `message_service` path (DMs, friend requests, call signaling).
    Legacy(Vec<u8>),
}

impl IngressItem {
    fn is_video(&self) -> bool {
        matches!(self, Self::Gossip { is_video: true, .. })
    }
}

/// FIFO queue between the Veilid dispatch loop and the gossip worker.
#[derive(Default)]
pub struct GossipIngressQueue {
    inner: Mutex<VecDeque<IngressItem>>,
    notify: Notify,
    /// Video items evicted/refused under overflow.
    pub video_drops: AtomicU64,
    /// Non-video items refused under overflow (pathological).
    pub other_drops: AtomicU64,
}

impl GossipIngressQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue an item, applying the drop-oldest-video overflow policy.
    pub fn push(&self, item: IngressItem) {
        {
            let mut q = self.inner.lock();
            if q.len() >= INGRESS_QUEUE_MAX {
                if let Some(pos) = q.iter().position(IngressItem::is_video) {
                    // Evict the OLDEST video item — stale media is the
                    // least valuable thing in the queue.
                    q.remove(pos);
                    self.video_drops.fetch_add(1, Ordering::Relaxed);
                } else if item.is_video() {
                    // No queued video to shed and the incoming item is
                    // video: drop the incoming (control keeps priority).
                    self.video_drops.fetch_add(1, Ordering::Relaxed);
                    return;
                } else {
                    // All-control queue AND control incoming: refuse the
                    // newest. This means processing has been stalled for
                    // hundreds of envelopes — surface it.
                    let n = self.other_drops.fetch_add(1, Ordering::Relaxed) + 1;
                    if n == 1 || n.is_multiple_of(50) {
                        tracing::warn!(
                            dropped_total = n,
                            "ingress queue full of control traffic — dropping inbound envelope"
                        );
                    }
                    return;
                }
            }
            q.push_back(item);
        }
        self.notify.notify_one();
    }

    /// Await the next item (FIFO). Cancel-safe: a `Notify` permit is
    /// only consumed when the queue really was empty.
    pub async fn pop(&self) -> IngressItem {
        loop {
            if let Some(item) = self.inner.lock().pop_front() {
                return item;
            }
            self.notify.notified().await;
        }
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.inner.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gossip(is_video: bool, tag: u8) -> IngressItem {
        IngressItem::Gossip {
            signed: SignedEnvelope {
                community_id: format!("c{tag}"),
                sender_pseudonym: "p".into(),
                envelope_bytes: vec![tag],
                signature: Vec::new(),
                ttl: 0,
            },
            is_video,
        }
    }

    fn tag_of(item: &IngressItem) -> u8 {
        match item {
            IngressItem::Gossip { signed, .. } => signed.envelope_bytes[0],
            IngressItem::Legacy(b) => b[0],
        }
    }

    #[test]
    fn fifo_order_preserved() {
        let q = GossipIngressQueue::new();
        q.push(gossip(false, 1));
        q.push(IngressItem::Legacy(vec![2]));
        q.push(gossip(true, 3));
        let mut got = Vec::new();
        while q.depth() > 0 {
            got.push(tag_of(&q.inner.lock().pop_front().unwrap()));
        }
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[test]
    fn overflow_evicts_oldest_video_first() {
        let q = GossipIngressQueue::new();
        // Fill: one old video item buried among control.
        q.push(gossip(false, 0));
        q.push(gossip(true, 1)); // ← oldest video
        for _ in 2..INGRESS_QUEUE_MAX {
            q.push(gossip(false, 2));
        }
        assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
        // Overflow with a control item: the old video is shed, the
        // control item lands.
        q.push(gossip(false, 9));
        assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
        assert_eq!(q.video_drops.load(Ordering::Relaxed), 1);
        assert_eq!(q.other_drops.load(Ordering::Relaxed), 0);
        let has_video = q.inner.lock().iter().any(IngressItem::is_video);
        assert!(!has_video, "the only video item must be the one evicted");
    }

    #[test]
    fn control_survives_video_flood() {
        let q = GossipIngressQueue::new();
        q.push(gossip(false, 7)); // the one control envelope
        for _ in 1..INGRESS_QUEUE_MAX {
            q.push(gossip(true, 1));
        }
        // Flood far past capacity with more video.
        for _ in 0..100 {
            q.push(gossip(true, 1));
        }
        assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
        assert_eq!(q.video_drops.load(Ordering::Relaxed), 100);
        let control_alive = q.inner.lock().iter().any(|i| !i.is_video());
        assert!(control_alive, "control envelope must outlive the flood");
    }

    #[test]
    fn all_control_overflow_refuses_newest() {
        let q = GossipIngressQueue::new();
        for _ in 0..INGRESS_QUEUE_MAX {
            q.push(gossip(false, 1));
        }
        // Incoming video against an all-control queue: incoming dropped.
        q.push(gossip(true, 2));
        assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
        assert_eq!(q.video_drops.load(Ordering::Relaxed), 1);
        // Incoming control against an all-control queue: incoming dropped.
        q.push(gossip(false, 3));
        assert_eq!(q.depth(), INGRESS_QUEUE_MAX);
        assert_eq!(q.other_drops.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn pop_wakes_on_push() {
        let q = std::sync::Arc::new(GossipIngressQueue::new());
        let q2 = std::sync::Arc::clone(&q);
        let waiter = tokio::spawn(async move { tag_of(&q2.pop().await) });
        // Give the waiter a moment to park on notified().
        tokio::task::yield_now().await;
        q.push(gossip(false, 42));
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("pop must wake")
            .expect("task join");
        assert_eq!(got, 42);
    }
}
