//! Shared event journal with cursor-based resumption.
//!
//! One journal per daemon process. All subscription events are appended
//! with a global monotonic sequence number. Each connection tracks its
//! own cursor (last delivered sequence). On reconnect, the client sends
//! `EventResume { last_seen_seq }` and the server replays missed events
//! from the journal.
//!
//! The journal is a bounded ring buffer. Old events are evicted when
//! the buffer wraps. A client whose cursor falls behind the oldest
//! available event must re-fetch full state.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;
use rekindle_types::subscription_events::SubscriptionEvent;

/// Default journal capacity: 65536 events.
const DEFAULT_CAPACITY: usize = 65536;

/// A sequenced event in the journal.
#[derive(Clone)]
pub struct SequencedEvent {
    pub seq: u64,
    pub event: Arc<SubscriptionEvent>,
}

/// Shared event journal. One per daemon. Thread-safe.
///
/// - Write path: `append()` — called by the event delivery task,
///   write-locks the entries.
/// - Read path: `replay_from()` — called by connection handlers
///   responding to `EventResume`, read-locks the entries.
///
/// At 100K connections, `replay_from` is called once per reconnecting
/// client. The read lock does not contend with other readers.
/// The write lock is held for one `push_back` + one conditional
/// `pop_front` — microseconds.
pub struct EventJournal {
    entries: RwLock<VecDeque<SequencedEvent>>,
    next_seq: AtomicU64,
    capacity: usize,
}

/// Error when the requested cursor is too old.
#[derive(Debug)]
pub struct JournalTruncated {
    pub oldest_available: u64,
    pub requested: u64,
}

impl std::fmt::Display for JournalTruncated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "journal truncated: requested seq {} but oldest available is {}",
            self.requested, self.oldest_available
        )
    }
}

impl std::error::Error for JournalTruncated {}

impl EventJournal {
    /// Create a new journal with default capacity.
    pub fn new() -> Arc<Self> {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Create a journal with a specific capacity.
    pub fn with_capacity(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            entries: RwLock::new(VecDeque::with_capacity(capacity)),
            next_seq: AtomicU64::new(0),
            capacity,
        })
    }

    /// Append an event to the journal. Returns the assigned sequence number.
    ///
    /// The event is wrapped in `Arc` for zero-copy sharing across
    /// replay responses to multiple reconnecting clients.
    pub fn append(&self, event: SubscriptionEvent) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let entry = SequencedEvent {
            seq,
            event: Arc::new(event),
        };

        let mut entries = self.entries.write();
        if entries.len() >= self.capacity {
            entries.pop_front();
        }
        entries.push_back(entry);
        seq
    }

    /// Replay events.
    ///
    /// `after_seq: None` — return all events from the beginning.
    /// `after_seq: Some(n)` — return events with seq > n.
    pub fn replay_from(&self, after_seq: Option<u64>) -> Result<Vec<SequencedEvent>, JournalTruncated> {
        let entries = self.entries.read();

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let oldest = entries.front().map_or(0, |e| e.seq);
        let newest = entries.back().map_or(0, |e| e.seq);

        match after_seq {
            None => Ok(entries.iter().cloned().collect()),
            Some(cursor) => {
                if cursor >= newest {
                    return Ok(Vec::new());
                }
                if cursor + 1 < oldest {
                    return Err(JournalTruncated {
                        oldest_available: oldest,
                        requested: cursor,
                    });
                }
                Ok(entries.iter().filter(|e| e.seq > cursor).cloned().collect())
            }
        }
    }

    /// Current head sequence number (the most recently assigned).
    pub fn head_seq(&self) -> u64 {
        let current = self.next_seq.load(Ordering::Relaxed);
        if current == 0 { 0 } else { current - 1 }
    }

    /// Number of events currently in the journal.
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Whether the journal is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }
}

static_assertions::assert_impl_all!(EventJournal: Send, Sync);

impl Default for EventJournal {
    fn default() -> Self {
        Self {
            entries: RwLock::new(VecDeque::with_capacity(DEFAULT_CAPACITY)),
            next_seq: AtomicU64::new(0),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::subscription_events::{SubscriptionEvent, UnreadContext};

    fn make_event(n: u32) -> SubscriptionEvent {
        SubscriptionEvent::UnreadChanged {
            context: UnreadContext::FriendRequests,
            count: n,
        }
    }

    #[test]
    fn append_and_replay() {
        let journal = EventJournal::new();
        journal.append(make_event(1));
        journal.append(make_event(2));
        journal.append(make_event(3));

        let replayed = journal.replay_from(Some(0)).unwrap();
        assert_eq!(replayed.len(), 2);
        assert_eq!(replayed[0].seq, 1);
        assert_eq!(replayed[1].seq, 2);
    }

    #[test]
    fn replay_from_beginning() {
        let journal = EventJournal::new();
        journal.append(make_event(1));
        journal.append(make_event(2));

        let all = journal.replay_from(Some(u64::MAX - 1)).unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn truncation_detected() {
        let journal = EventJournal::with_capacity(4);

        for i in 0..10 {
            journal.append(make_event(i));
        }

        assert_eq!(journal.len(), 4);
        let result = journal.replay_from(Some(2));
        assert!(result.is_err());
    }

    #[test]
    fn empty_journal_replay() {
        let journal = EventJournal::new();
        let replayed = journal.replay_from(Some(0)).unwrap();
        assert!(replayed.is_empty());
    }

    #[test]
    fn replay_from_none_returns_all() {
        let journal = EventJournal::new();
        journal.append(make_event(1));
        journal.append(make_event(2));
        journal.append(make_event(3));
        let all = journal.replay_from(None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].seq, 0);
        assert_eq!(all[1].seq, 1);
        assert_eq!(all[2].seq, 2);
    }
}
