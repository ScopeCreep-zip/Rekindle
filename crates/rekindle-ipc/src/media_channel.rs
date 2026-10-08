//! Bounded, drop-oldest queue for daemon→client media frames.
//!
//! Media (video frames) must not ride the ordered, deduped, journaled event
//! stream (`BusPayload::Event`), and a late frame is worthless — so the media
//! path uses this queue, which evicts the **oldest** item when full rather
//! than blocking the producer or dropping the newest. It mirrors the
//! drop-oldest semantics of the webview `VideoChannelRegistry`
//! (`src-tauri/src/video_channels.rs`), but here the transport is the postcard
//! Noise bus, not a Tauri channel.
//!
//! Two bounded hops use it:
//! - **server → connection**: the per-connection fan-out queue that drains to
//!   one client's socket (`T = Vec<u8>`, an encoded `Message<BusPayload>`).
//! - **client inbound → consumer**: the queue `take_media_receiver` hands to
//!   the TUI/CLI (`T = MediaFrame`).
//!
//! A single-producer/single-consumer discipline is expected (the sender is
//! `Clone` only so it can be moved into a task and still observe closure), and
//! the queue never blocks: `send` returns immediately, evicting the oldest
//! frame when the capacity is reached.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;

/// Shared inner state of a drop-oldest media queue.
struct Shared<T> {
    /// The bounded ring. `parking_lot::Mutex` — never held across an `.await`.
    queue: Mutex<VecDeque<T>>,
    /// Wakes the receiver when an item is enqueued or the last sender drops.
    notify: Notify,
    /// Maximum buffered items; the oldest is evicted on overflow.
    capacity: usize,
    /// Lifetime count of items evicted (oldest-dropped) — diagnostics only.
    dropped: AtomicU64,
    /// Live sender count. When it reaches zero the receiver observes closure.
    senders: AtomicUsize,
}

/// Producer half of a bounded drop-oldest media queue.
pub struct MediaSender<T> {
    shared: Arc<Shared<T>>,
}

/// Consumer half of a bounded drop-oldest media queue. Single consumer.
pub struct MediaReceiver<T> {
    shared: Arc<Shared<T>>,
}

/// Create a bounded drop-oldest queue with room for `capacity` items
/// (clamped to a minimum of 1).
pub fn media_channel<T>(capacity: usize) -> (MediaSender<T>, MediaReceiver<T>) {
    let capacity = capacity.max(1);
    let shared = Arc::new(Shared {
        queue: Mutex::new(VecDeque::with_capacity(capacity)),
        notify: Notify::new(),
        capacity,
        dropped: AtomicU64::new(0),
        senders: AtomicUsize::new(1),
    });
    (
        MediaSender {
            shared: Arc::clone(&shared),
        },
        MediaReceiver { shared },
    )
}

impl<T> MediaSender<T> {
    /// Enqueue an item, evicting the oldest if the queue is at capacity.
    ///
    /// Never blocks and never fails: a late frame is worthless, so on overflow
    /// the queue drops the oldest buffered frame to make room for this one.
    /// Returns `true` if an older frame was evicted.
    pub fn send(&self, item: T) -> bool {
        let evicted = {
            let mut q = self.shared.queue.lock();
            let evicted = if q.len() >= self.shared.capacity {
                q.pop_front();
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                true
            } else {
                false
            };
            q.push_back(item);
            evicted
        };
        // Notify AFTER releasing the lock — never hold a parking_lot guard
        // across a wake-up.
        self.shared.notify.notify_one();
        evicted
    }

    /// Total items evicted (oldest-dropped) over this queue's lifetime.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}

impl<T> Clone for MediaSender<T> {
    fn clone(&self) -> Self {
        self.shared.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for MediaSender<T> {
    fn drop(&mut self) {
        // Last sender gone — wake the receiver so it can observe closure and
        // return `None` once the queue drains.
        if self.shared.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.notify.notify_one();
        }
    }
}

impl<T> MediaReceiver<T> {
    /// Await the next item. Returns `None` once every sender has dropped and
    /// the queue has fully drained.
    ///
    /// A frame that arrived while the consumer was busy may already have been
    /// evicted (drop-oldest) — a late frame is dropped, never queued
    /// unboundedly.
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            // Register interest BEFORE the empty-check so a concurrent `send`
            // between the check and the await cannot be lost: `notify_one`
            // stores a permit the next `notified().await` consumes.
            let notified = self.shared.notify.notified();
            {
                let mut q = self.shared.queue.lock();
                if let Some(item) = q.pop_front() {
                    return Some(item);
                }
                if self.shared.senders.load(Ordering::Acquire) == 0 {
                    return None;
                }
            }
            notified.await;
        }
    }

    /// Non-blocking dequeue: the next item, or `None` if the queue is empty.
    pub fn try_recv(&mut self) -> Option<T> {
        self.shared.queue.lock().pop_front()
    }

    /// Current buffered item count (diagnostics/tests).
    pub fn len(&self) -> usize {
        self.shared.queue.lock().len()
    }

    /// Whether the queue currently holds no items.
    pub fn is_empty(&self) -> bool {
        self.shared.queue.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn send_then_recv_preserves_order() {
        let (tx, mut rx) = media_channel::<u32>(8);
        for i in 0..5 {
            assert!(!tx.send(i), "no eviction below capacity");
        }
        for i in 0..5 {
            assert_eq!(rx.recv().await, Some(i));
        }
        assert_eq!(tx.dropped(), 0);
    }

    #[tokio::test]
    async fn overflow_drops_oldest_keeps_newest() {
        let cap = 4;
        let (tx, mut rx) = media_channel::<u32>(cap);
        // Push 10 items into a queue of capacity 4: the 6 oldest are evicted.
        let overflow = 6u32;
        let total = u32::try_from(cap).unwrap() + overflow;
        for i in 0..total {
            tx.send(i);
        }
        assert_eq!(
            tx.dropped(),
            u64::from(overflow),
            "oldest evicted on overflow"
        );
        assert_eq!(rx.len(), cap, "queue never exceeds capacity");

        // What remains is the NEWEST `cap` items, in order.
        let mut got = Vec::new();
        while let Some(v) = rx.try_recv() {
            got.push(v);
        }
        assert_eq!(
            got,
            vec![overflow, overflow + 1, overflow + 2, overflow + 3]
        );
    }

    #[tokio::test]
    async fn recv_returns_none_after_all_senders_drop() {
        let (tx, mut rx) = media_channel::<u32>(2);
        tx.send(1);
        tx.send(2);
        drop(tx);
        // Buffered items drain first, then closure is observed.
        assert_eq!(rx.recv().await, Some(1));
        assert_eq!(rx.recv().await, Some(2));
        assert_eq!(rx.recv().await, None);
    }

    #[tokio::test]
    async fn recv_wakes_on_concurrent_send() {
        let (tx, mut rx) = media_channel::<u32>(4);
        let handle = tokio::spawn(async move { rx.recv().await });
        // Give the receiver a moment to park on an empty queue.
        tokio::task::yield_now().await;
        tx.send(42);
        assert_eq!(handle.await.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn clone_keeps_channel_open_until_last_sender_drops() {
        let (tx, mut rx) = media_channel::<u32>(2);
        let tx2 = tx.clone();
        tx.send(7);
        drop(tx);
        // A live clone keeps the channel open.
        assert_eq!(rx.recv().await, Some(7));
        tx2.send(8);
        drop(tx2);
        assert_eq!(rx.recv().await, Some(8));
        assert_eq!(rx.recv().await, None);
    }
}
