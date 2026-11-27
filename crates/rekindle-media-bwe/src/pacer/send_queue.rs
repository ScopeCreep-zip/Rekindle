//! Copied from str0m `src/streams/send_queue.rs`.
//!
//! str0m's queue holds `RtpPacket`s and reads their `timestamp` field and payload
//! length. Here the queue is generic over the packet type: each entry carries the
//! timestamp and size beside the caller's packet.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::pacer::{QueuePriority, QueueSnapshot};
use crate::util::{not_happening, usize_as_u32};

/// One queued packet: str0m's `RtpPacket.timestamp` and `payload.len()`, beside
/// the caller's packet.
#[derive(Debug)]
struct Queued<T> {
    timestamp: Instant,
    size: usize,
    packet: T,
}

/// A FIFO of packets waiting for the pacer, with the running totals the pacer's
/// [`QueueSnapshot`] needs.
///
/// Use: [`push`](Self::push) a packet, call [`handle_timeout`](Self::handle_timeout)
/// to timestamp it (str0m timestamps queued packets on the next timeout, never at
/// push), hand [`snapshot`](Self::snapshot) to the pacer, and [`pop`](Self::pop)
/// when the pacer names this queue.
#[derive(Debug)]
pub struct SendQueue<T> {
    queue: VecDeque<Queued<T>>,
    total: TotalQueue,
    last_emitted: Option<Instant>,
}

impl<T> Default for SendQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SendQueue<T> {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            total: TotalQueue::default(),
            last_emitted: None,
        }
    }

    /// Enqueue `packet`, `size` bytes long. It is not released before the next
    /// [`handle_timeout`](Self::handle_timeout) timestamps it.
    pub fn push(&mut self, packet: T, size: usize) {
        // Every incoming packet must be timestamped withe a handle_timeout.
        // This sentinel value indicates it is needed.
        self.queue.push_back(Queued {
            timestamp: not_happening(),
            size,
            packet,
        });
    }

    /// Timestamp every packet pushed since the last call.
    pub fn handle_timeout(&mut self, now: Instant) {
        for pkt in self.queue.iter_mut().rev() {
            if pkt.timestamp != not_happening() {
                // all enqueued packets are timestamped.
                break;
            }
            pkt.timestamp = now;
            self.total.increase(now, pkt.size);
        }
    }

    /// Whether a packet is waiting for [`handle_timeout`](Self::handle_timeout).
    pub fn need_timeout(&self) -> bool {
        // Packets are timestamped contiguously from the front,
        // so checking the tail summarizes the entire queue's state.
        self.queue
            .back()
            .is_some_and(|p| p.timestamp == not_happening())
    }

    /// The next packet to release, if it has been timestamped.
    pub fn peek(&mut self) -> Option<&mut T> {
        let peeked = self.queue.front_mut()?;
        if peeked.timestamp == not_happening() {
            None
        } else {
            Some(&mut peeked.packet)
        }
    }

    /// Release the next packet, if it has been timestamped.
    pub fn pop(&mut self, now: Instant) -> Option<T> {
        // Don't release packets without a timestamp.
        self.peek()?;

        // peek() above must have returned a value for us to be here.
        let packet = self.queue.pop_front()?;

        // Must be timestamped
        assert!(packet.timestamp != not_happening());

        let queue_time = now - packet.timestamp;
        self.total.decrease(now, packet.size, queue_time);
        self.last_emitted = Some(now);

        Some(packet.packet)
    }

    /// Whether the queue holds no packets.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The most recently pushed packet.
    pub fn last(&self) -> Option<&T> {
        self.queue.back().map(|q| &q.packet)
    }

    /// The queue's state for the pacer at `now`.
    pub fn snapshot(&mut self, now: Instant) -> QueueSnapshot {
        self.total.move_time_forward(now);

        QueueSnapshot {
            created_at: now,
            byte_size: self.total.unsent_size,
            packet_count: usize_as_u32(self.total.unsent_count),
            total_queue_time_origin: self.total.queue_time,
            last_emitted: self.last_emitted,
            first_unsent: self
                .queue
                .iter()
                .find(|p| p.timestamp != not_happening())
                .map(|p| p.timestamp),
            priority: if self.total.unsent_count > 0 {
                QueuePriority::Media
            } else {
                QueuePriority::Empty
            },
        }
    }

    /// Drop every packet and reset the totals.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.total.clear();
        self.last_emitted = None;
    }
}

// Total queue time in buffer. This lovely drawing explains how to add more time.
//
// -time--------------------------------------------------------->
//
// +--------------+
// |              |
// +--------------+
//      +---------+                          Already
//      |         |                           queued
//      +---------+                         durations
//          +-----+
//          |     |
//          +-----+
//                       +-+
//                       | |         <-----  Add next
//                       +-+                  packet
//
//
//
// +--------------+--------+
// |              |@@@@@@@@|
// +--------------+--------+
//      +---------+--------+                 The @ is
//      |         |@@@@@@@@|                  what's
//      +---------+--------+                  added
//          +-----+--------+
//          |     |@@@@@@@@|
//          +-----+--------+
//                       +-+
//                       |@|
//                       +-+
#[derive(Debug, Default)]
struct TotalQueue {
    /// Number of unsent packets.
    unsent_count: usize,
    /// The data size (bytes) of the unsent packets.
    unsent_size: usize,
    // /// When we last added some value to `queue_time`.
    // last: Option<Instant>,
    /// The total queue time of all the unsent packets.
    queue_time: Duration,
    // Timestamp of the last added packet.
    last: Option<Instant>,
}

impl TotalQueue {
    fn move_time_forward(&mut self, now: Instant) {
        if let Some(last) = self.last {
            assert!(self.unsent_count > 0);
            let from_last = now - last;
            self.queue_time += from_last * usize_as_u32(self.unsent_count);
            self.last = Some(now);
        } else {
            assert!(self.unsent_count == 0);
            assert!(self.unsent_size == 0);
            assert!(self.queue_time == Duration::ZERO);
        }
    }

    fn increase(&mut self, now: Instant, size: usize) {
        self.move_time_forward(now);
        self.unsent_count += 1;
        self.unsent_size += size;
        self.last = Some(now);
    }

    fn decrease(&mut self, now: Instant, size: usize, queue_time: Duration) {
        self.move_time_forward(now);

        self.unsent_count -= 1;
        self.unsent_size -= size;

        self.queue_time -= queue_time;

        if self.unsent_count == 0 {
            assert!(self.unsent_size == 0);
            self.queue_time = Duration::ZERO;
            self.last = None;
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
#[path = "send_queue_test.rs"]
mod test;
