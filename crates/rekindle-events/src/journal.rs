//! Bounded in-memory event journal with monotonic cursors.
//!
//! Every emitted event gets a unique [`JournalCursor`]. The frontend
//! persists the most-recent cursor to `localStorage`; on soft stalls
//! (page reload during dev, brief IPC pause, hot-reload across the
//! Tauri ↔ webview bridge) it calls the `event_resume` Tauri command
//! which returns [`EventJournal::replay_since`] — events newer than the
//! cursor in arrival order.
//!
//! ## Persistence
//!
//! The journal is **in-memory only** — the [`AtomicU64`] cursor counter
//! resets to `1` on every process start and the ring buffer is empty
//! after `kill -9` + restart. Cold-start recovery for persisted state
//! (DMs, community messages, friend requests) goes through their
//! respective SQLite stores via the existing message-history Tauri
//! commands; the journal handles the gap between "the event fired"
//! and "the frontend's live listener was installed" within a single
//! process lifetime.
//!
//! Phase 10 generalised the journal over `T` so the daemon track can
//! store `SubscriptionEvent` while the Tauri track stores its own
//! `{ channel, payload }` envelope. Either way the cursor + ring semantics
//! are identical.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;

pub type JournalCursor = u64;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "T: serde::Serialize",
    deserialize = "T: serde::de::DeserializeOwned",
))]
pub struct JournalEntry<T> {
    pub cursor: JournalCursor,
    pub event: T,
}

pub struct EventJournal<T> {
    next: AtomicU64,
    ring: RwLock<VecDeque<JournalEntry<T>>>,
    capacity: usize,
}

impl<T> EventJournal<T> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            next: AtomicU64::new(1),
            ring: RwLock::new(VecDeque::with_capacity(capacity.min(128))),
            capacity,
        }
    }

    pub fn append(&self, event: T) -> JournalCursor {
        let cursor = self.next.fetch_add(1, Ordering::Relaxed);
        let mut g = self.ring.write();
        if g.len() >= self.capacity {
            g.pop_front();
        }
        g.push_back(JournalEntry { cursor, event });
        cursor
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ring.read().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ring.read().is_empty()
    }
}

impl<T: Clone> EventJournal<T> {
    pub fn replay_since(&self, since: JournalCursor) -> Vec<JournalEntry<T>> {
        self.ring
            .read()
            .iter()
            .filter(|e| e.cursor > since)
            .cloned()
            .collect()
    }
}

impl<T> EventJournal<T> {
    /// Empty the ring buffer **and** reset the cursor counter to 1.
    ///
    /// Called on logout so a subsequent login (same identity or
    /// otherwise) cannot replay events that belonged to the previous
    /// session — a privacy hazard for a vulnerable-users platform
    /// where shared devices are common. Resetting the counter is
    /// intentional: any cursor the frontend has persisted to
    /// `localStorage` is now meaningless against the new journal
    /// generation, but cross-session resume is best-effort and the
    /// frontend treats empty replays as the cold-start default.
    pub fn clear(&self) {
        self.ring.write().clear();
        self.next.store(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::subscription_events::{NetworkEvent, SubscriptionEvent};

    fn ev() -> SubscriptionEvent {
        SubscriptionEvent::Network(NetworkEvent::AttachmentChanged {
            is_attached: true,
            public_internet_ready: true,
        })
    }

    #[test]
    fn cursors_are_monotonic() {
        let j: EventJournal<SubscriptionEvent> = EventJournal::new(10);
        let c1 = j.append(ev());
        let c2 = j.append(ev());
        assert_eq!(c2, c1 + 1);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let j: EventJournal<SubscriptionEvent> = EventJournal::new(2);
        let _c1 = j.append(ev());
        let _c2 = j.append(ev());
        let _c3 = j.append(ev());
        assert_eq!(j.len(), 2);
    }

    #[test]
    fn replay_since_returns_newer_only() {
        let j: EventJournal<SubscriptionEvent> = EventJournal::new(10);
        let c1 = j.append(ev());
        let _c2 = j.append(ev());
        let _c3 = j.append(ev());
        let out = j.replay_since(c1);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|entry| entry.cursor > c1));
    }

    /// Phase 10 — the journal must also work with non-`SubscriptionEvent`
    /// payloads (the Tauri track uses `serde_json::Value`-shaped records).
    #[test]
    fn generic_payload_round_trips() {
        let j: EventJournal<String> = EventJournal::new(4);
        j.append("a".into());
        j.append("b".into());
        let out = j.replay_since(0);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].event, "a");
        assert_eq!(out[1].event, "b");
    }

    /// Phase 10 — `clear()` drops all entries AND resets the cursor so
    /// post-logout replay can't surface previous-session events. This
    /// is the privacy invariant the logout path depends on.
    #[test]
    fn clear_empties_ring_and_resets_cursor() {
        let j: EventJournal<String> = EventJournal::new(8);
        j.append("alice-secret-1".into());
        j.append("alice-secret-2".into());
        assert_eq!(j.len(), 2);
        j.clear();
        assert!(j.is_empty(), "ring must be empty after clear");
        // Cursor restarts at 1 — a stale localStorage cursor from the
        // previous session would now be GREATER than any cursor the
        // new session will produce, so replay returns nothing.
        let c = j.append("bob-fresh-1".into());
        assert_eq!(c, 1, "post-clear cursor must restart at 1");
        let leaked = j.replay_since(0);
        assert_eq!(leaked.len(), 1, "only the post-clear event is visible");
        assert_eq!(leaked[0].event, "bob-fresh-1");
        assert!(
            leaked.iter().all(|e| !e.event.contains("alice")),
            "no alice-* events may survive into bob's session",
        );
    }
}
