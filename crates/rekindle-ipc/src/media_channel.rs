//! Bounded, drop-oldest queue for media frames, both directions (plan E4.1).
//!
//! Media (video frames) must not ride the ordered, deduped, journaled event
//! stream (`BusPayload::Event`), and a late frame is worthless — so the media
//! path uses this queue, which evicts the **oldest** item when full rather
//! than blocking the producer or dropping the newest. It mirrors the
//! drop-oldest semantics of the webview `VideoChannelRegistry`
//! (`src-tauri/src/video_channels.rs`), but here the transport is the postcard
//! Noise bus, not a Tauri channel.
//!
//! The queue backs these media lanes:
//! - **client → socket**: encoded bus media waits apart from control traffic.
//! - **server → socket**: uplink to the daemon and downlink to subscribers.
//! - **client inbound → consumer**: typed `MediaFrame` and `UplinkFrame`
//!   receivers exposed by `BusClient`.
//! - **in-process Host lanes**: `HostApi::media_up`/`media_down`.
//!
//! The queue has one receiver and supports cloned senders. Producers never
//! await space. A full queue evicts its oldest frame.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
    /// Cleared when the receiver drops, so a fan-out prunes the sender.
    receiver_alive: AtomicBool,
}

/// Maximum queued encoded frames per media hop.
pub const MEDIA_LANE_CAPACITY: usize = 128;

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
        receiver_alive: AtomicBool::new(true),
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
    /// Returns `true` if an older frame was evicted. A closed queue discards the item.
    pub fn send(&self, item: T) -> bool {
        let evicted = {
            let mut q = self.shared.queue.lock();
            if self.is_closed() {
                return false;
            }
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

    /// Replace the oldest buffered item matching `same_source`. If that
    /// source has no backlog, discard the incoming item.
    pub(crate) fn replace_oldest(&self, item: T, same_source: impl FnMut(&T) -> bool) {
        let mut queue = self.shared.queue.lock();
        if let Some(index) = queue.iter().position(same_source) {
            queue.remove(index);
            queue.push_back(item);
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Whether the receiver has dropped: nothing sent now is ever read.
    pub fn is_closed(&self) -> bool {
        !self.shared.receiver_alive.load(Ordering::Acquire)
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

impl<T> Drop for MediaReceiver<T> {
    fn drop(&mut self) {
        let mut queue = self.shared.queue.lock();
        self.shared.receiver_alive.store(false, Ordering::Release);
        queue.clear();
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
#[path = "media_channel_tests.rs"]
mod tests;
