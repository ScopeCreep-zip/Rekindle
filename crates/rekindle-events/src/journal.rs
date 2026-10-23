//! The host's one event journal (plan D21, D3.2).
//!
//! Every `SubscriptionEvent` the host publishes is appended under a
//! [`Cursor`] and sent to the live subscribers. An unlock and a lock each
//! begin a new epoch: the ring is emptied (one identity's events never
//! replay into another session) and every live subscriber is told
//! [`StreamFrame::InvalidSession`].
//!
//! A subscription is taken under the journal's lock, which `publish` holds
//! while it appends and sends, so the replay and the live stream neither
//! overlap nor leave a gap. Every queue is bounded: a subscriber that falls
//! more than the live capacity behind is sent `InvalidSession` and its
//! stream ends, so it reloads instead of silently missing events.

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_types::subscription_events::{Cursor, Sequenced, StreamFrame, SubscriptionEvent};
use rekindle_utils::RandomSource;
use tokio::sync::broadcast;

/// How many events a reconnecting subscriber can catch up on.
pub const RING_CAPACITY: usize = 10_000;

/// How far a live subscriber may fall behind before it must reload.
pub const LIVE_CAPACITY: usize = 4_096;

struct Ring {
    epoch: u64,
    /// The sequence number the next event gets.
    next: u64,
    events: VecDeque<Sequenced>,
}

/// The host's journal. One per host; its epoch changes with each unlock.
pub struct SessionJournal {
    ring: RwLock<Ring>,
    live: broadcast::Sender<StreamFrame>,
    capacity: usize,
    /// The host's RNG (plan C9.2): each epoch is drawn from it.
    rng: Arc<dyn RandomSource>,
}

/// A fresh random epoch below 2^53 (exact as a JavaScript number).
fn new_epoch(rng: &dyn RandomSource) -> u64 {
    let mut bytes = [0u8; 8];
    rng.fill(&mut bytes);
    u64::from_le_bytes(bytes) >> 11
}

impl SessionJournal {
    /// The host's journal at the standard capacities, its epochs drawn
    /// from `rng`, the host's RandomSource.
    #[must_use]
    pub fn with_rng(rng: Arc<dyn RandomSource>) -> Self {
        Self::new(RING_CAPACITY, LIVE_CAPACITY, rng)
    }

    /// A journal keeping `capacity` events for replay, whose live
    /// subscribers may fall `live_capacity` frames behind, its epochs
    /// drawn from `rng`.
    #[must_use]
    pub fn new(capacity: usize, live_capacity: usize, rng: Arc<dyn RandomSource>) -> Self {
        Self {
            ring: RwLock::new(Ring {
                epoch: new_epoch(rng.as_ref()),
                next: 1,
                events: VecDeque::with_capacity(capacity.min(1024)),
            }),
            live: broadcast::channel(live_capacity).0,
            capacity,
            rng,
        }
    }

    /// Begin a new epoch (unlock and lock): empty the ring and tell every
    /// live subscriber its cursor is no longer valid.
    pub fn begin(&self) {
        let mut ring = self.ring.write();
        ring.epoch = new_epoch(self.rng.as_ref());
        ring.next = 1;
        ring.events.clear();
        // No subscriber is not an error.
        let _ = self.live.send(StreamFrame::InvalidSession);
    }

    /// Journal `event` and send it to the live subscribers.
    pub fn publish(&self, event: SubscriptionEvent) -> Cursor {
        let mut ring = self.ring.write();
        let cursor = Cursor {
            epoch: ring.epoch,
            seq: ring.next,
        };
        ring.next += 1;
        if ring.events.len() >= self.capacity {
            ring.events.pop_front();
        }
        let sequenced = Sequenced { cursor, event };
        ring.events.push_back(sequenced.clone());
        let _ = self.live.send(StreamFrame::Event(sequenced));
        cursor
    }

    /// The current position: the cursor of the last event published in this
    /// epoch (seq 0 before the first).
    #[must_use]
    pub fn head(&self) -> Cursor {
        let ring = self.ring.read();
        Cursor {
            epoch: ring.epoch,
            seq: ring.next - 1,
        }
    }

    /// Subscribe from `from`: `None` for live events only; otherwise
    /// [`Self::replay`]'s frames first, then live.
    #[must_use]
    pub fn subscribe(&self, from: Option<Cursor>) -> EventStream {
        let ring = self.ring.read();
        let live = self.live.subscribe();
        let first = from
            .map(|from| replay_from(&ring, from))
            .unwrap_or_default();
        EventStream {
            source: Source::Journal {
                first: first.into(),
                live: Some(live),
            },
        }
    }

    /// What followed `from` and `Resumed` when `from` is this epoch's and the
    /// ring still covers it; otherwise `InvalidSession`. For a consumer that
    /// takes live frames elsewhere (the GUI's windows share one live stream)
    /// and so dedupes by cursor.
    #[must_use]
    pub fn replay(&self, from: Cursor) -> Vec<StreamFrame> {
        replay_from(&self.ring.read(), from)
    }
}

fn replay_from(ring: &Ring, from: Cursor) -> Vec<StreamFrame> {
    let head = ring.next - 1;
    let oldest = ring.events.front().map_or(ring.next, |e| e.cursor.seq);
    // Served when the cursor is this epoch's, not ahead of the head, and
    // nothing after it was evicted.
    if from.epoch == ring.epoch && from.seq <= head && from.seq + 1 >= oldest {
        let mut frames: Vec<StreamFrame> = ring
            .events
            .iter()
            .filter(|e| e.cursor.seq > from.seq)
            .cloned()
            .map(StreamFrame::Event)
            .collect();
        frames.push(StreamFrame::Resumed(Cursor {
            epoch: ring.epoch,
            seq: head,
        }));
        frames
    } else {
        vec![StreamFrame::InvalidSession]
    }
}

/// One subscriber's stream: its replay, then the live frames — from this
/// process's journal, or from a channel another transport feeds (the
/// daemon client's bus stream, plan E1.1).
pub struct EventStream {
    source: Source,
}

enum Source {
    Journal {
        first: VecDeque<StreamFrame>,
        live: Option<broadcast::Receiver<StreamFrame>>,
    },
    Channel(tokio::sync::mpsc::Receiver<StreamFrame>),
}

impl EventStream {
    /// A stream fed by `frames`: the frames as the sender orders them (a
    /// bus client relays the host's, replay and `Resumed` included).
    #[must_use]
    pub fn from_channel(frames: tokio::sync::mpsc::Receiver<StreamFrame>) -> Self {
        Self {
            source: Source::Channel(frames),
        }
    }

    /// The next frame, or `None` once the stream has ended (the journal or
    /// the channel is gone, or the subscriber fell behind and was sent
    /// `InvalidSession`).
    pub async fn next(&mut self) -> Option<StreamFrame> {
        match &mut self.source {
            Source::Channel(frames) => frames.recv().await,
            Source::Journal { first, live } => {
                if let Some(frame) = first.pop_front() {
                    return Some(frame);
                }
                let receiver = live.as_mut()?;
                match receiver.recv().await {
                    Ok(frame) => Some(frame),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "event subscriber fell behind; it reloads");
                        *live = None;
                        Some(StreamFrame::InvalidSession)
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        *live = None;
                        None
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::subscription_events::NetworkEvent;

    fn ev(key: &str) -> SubscriptionEvent {
        SubscriptionEvent::Network(NetworkEvent::ValueChanged {
            record_key: key.into(),
            changed_subkeys: vec![],
        })
    }

    fn seq_of(frame: &StreamFrame) -> Option<u64> {
        match frame {
            StreamFrame::Event(s) => Some(s.cursor.seq),
            _ => None,
        }
    }

    #[tokio::test]
    async fn a_channel_stream_yields_its_sender_s_frames_then_ends() {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let mut stream = EventStream::from_channel(rx);
        tx.send(StreamFrame::InvalidSession).await.unwrap();
        drop(tx);
        assert!(matches!(
            stream.next().await,
            Some(StreamFrame::InvalidSession)
        ));
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn a_resumed_subscriber_gets_what_it_missed_then_live_with_no_gap() {
        let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
        let first = journal.publish(ev("a"));
        journal.publish(ev("b"));
        journal.publish(ev("c"));
        let mut stream = journal.subscribe(Some(first));
        journal.publish(ev("d"));
        assert_eq!(seq_of(&stream.next().await.unwrap()), Some(2));
        assert_eq!(seq_of(&stream.next().await.unwrap()), Some(3));
        assert!(matches!(
            stream.next().await.unwrap(),
            StreamFrame::Resumed(Cursor { seq: 3, .. })
        ));
        assert_eq!(seq_of(&stream.next().await.unwrap()), Some(4));
    }

    #[tokio::test]
    async fn an_evicted_or_foreign_cursor_is_an_invalid_session() {
        let journal = SessionJournal::new(2, 16, Arc::new(rekindle_utils::random::OsRandom));
        let gone = journal.publish(ev("a"));
        journal.publish(ev("b"));
        journal.publish(ev("c"));
        journal.publish(ev("d"));
        assert!(matches!(
            journal.subscribe(Some(gone)).next().await,
            Some(StreamFrame::InvalidSession)
        ));
        let foreign = Cursor {
            epoch: journal.head().epoch ^ 1,
            seq: 1,
        };
        assert!(matches!(
            journal.subscribe(Some(foreign)).next().await,
            Some(StreamFrame::InvalidSession)
        ));
    }

    #[tokio::test]
    async fn a_new_epoch_invalidates_live_subscribers_and_empties_the_ring() {
        let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
        let before = journal.publish(ev("a"));
        let mut live = journal.subscribe(None);
        journal.begin();
        assert!(matches!(
            live.next().await,
            Some(StreamFrame::InvalidSession)
        ));
        assert_ne!(journal.head().epoch, before.epoch);
        assert_eq!(journal.head().seq, 0);
        assert!(matches!(
            journal.subscribe(Some(before)).next().await,
            Some(StreamFrame::InvalidSession)
        ));
    }

    #[tokio::test]
    async fn a_subscriber_that_falls_behind_reloads_and_its_stream_ends() {
        let journal = SessionJournal::new(64, 2, Arc::new(rekindle_utils::random::OsRandom));
        let mut live = journal.subscribe(None);
        for i in 0..8 {
            journal.publish(ev(&i.to_string()));
        }
        assert!(matches!(
            live.next().await,
            Some(StreamFrame::InvalidSession)
        ));
        assert!(live.next().await.is_none());
    }

    #[test]
    fn epochs_are_exact_in_javascript() {
        for _ in 0..64 {
            assert!(new_epoch(&rekindle_utils::random::OsRandom) < (1 << 53));
        }
    }
}
