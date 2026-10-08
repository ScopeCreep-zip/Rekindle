//! A packet queue for use in tests of the pacer (str0m `src/pacer/leaky.rs`, `mod test::queue`).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::util::usize_as_i64;
use crate::DataSize;

use super::*;

// A packet queue
pub(super) struct Queue {
    /// Queue for audio packets
    audio: Inner,
    /// Queue for video packets
    video: Inner,
    /// Queue for padding packets
    padding: Inner,
}

pub(super) struct QueuedPacket {
    pub(super) queued_at: Instant,
    pub(super) header: RtpHeader,
    pub(super) payload_len: usize,
    pub(super) kind: PacketKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PacketKind {
    Audio,
    Video,
    Padding,
}

impl Queue {
    pub(super) fn is_empty(&self) -> bool {
        self.audio.is_empty() && self.video.is_empty()
    }

    pub(super) fn update_average_queue_time(&mut self, now: Instant) {
        self.audio.update_average_queue_time(now);
        self.video.update_average_queue_time(now);
    }

    pub(super) fn enqueue_packet(&mut self, packet: QueuedPacket) {
        let queue = self.queue_for_kind_mut(packet.kind);
        queue.enqueue(packet);
    }

    pub(super) fn next_packet(&mut self) -> Option<QueuedPacket> {
        if !self.audio.is_empty() {
            self.audio.pop_packet()
        } else if !self.video.is_empty() {
            self.video.pop_packet()
        } else {
            self.padding.pop_packet()
        }
    }

    pub(super) fn queue_state(&self, now: Instant) -> impl Iterator<Item = QueueState> {
        vec![
            self.audio.queue_state(now),
            self.video.queue_state(now),
            self.padding.queue_state(now),
        ]
        .into_iter()
    }

    pub(super) fn register_send(&mut self, queue_id: QueueId, now: Instant) {
        if self.video.queue_id == queue_id {
            self.video.last_emitted = Some(now);
        } else if self.audio.queue_id == queue_id {
            self.audio.last_emitted = Some(now);
        } else if self.padding.queue_id == queue_id {
            self.padding.last_emitted = Some(now);
        } else {
            panic!("Attempted to register send on unknown queue with id {queue_id:?}");
        }
    }

    pub(super) fn generate_padding(&mut self, mut pad_size: usize, now: Instant) {
        while pad_size > 0 {
            let final_packet_size = pad_size.min(1200);
            let final_packet_size = DataSize::bytes(usize_as_i64(final_packet_size));
            let (header, payload_len, kind) =
                make_packet(0, final_packet_size.as_bytes_usize(), PacketKind::Padding);
            self.enqueue_packet(QueuedPacket {
                queued_at: now,
                header,
                payload_len,
                kind,
            });
            self.update_average_queue_time(now);

            pad_size = pad_size.saturating_sub(final_packet_size.as_bytes_usize());
        }
    }

    fn queue_for_kind_mut(&mut self, kind: PacketKind) -> &mut Inner {
        match kind {
            PacketKind::Audio => &mut self.audio,
            PacketKind::Video => &mut self.video,
            PacketKind::Padding => &mut self.padding,
        }
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            audio: Inner::new(QueueId(1), true, QueuePriority::Media),
            video: Inner::new(QueueId(2), false, QueuePriority::Media),
            padding: Inner::new(QueueId(3), false, QueuePriority::Padding),
        }
    }
}

impl QueuedPacket {
    pub(super) fn size(&self) -> usize {
        self.payload_len
    }
}

struct Inner {
    queue_id: QueueId,
    last_emitted: Option<Instant>,
    queue: VecDeque<QueuedPacket>,
    packet_count: u32,
    total_time_spent_queued: Duration,
    last_update: Option<Instant>,
    is_audio: bool,
    priority: QueuePriority,
}

impl Inner {
    fn new(queue_id: QueueId, is_audio: bool, priority: QueuePriority) -> Self {
        Self {
            queue_id,
            last_emitted: None,
            queue: VecDeque::default(),
            packet_count: 0,
            total_time_spent_queued: Duration::ZERO,
            last_update: None,
            is_audio,
            priority,
        }
    }

    fn enqueue(&mut self, packet: QueuedPacket) {
        self.queue.push_back(packet);
        self.packet_count += 1;
    }

    fn pop_packet(&mut self) -> Option<QueuedPacket> {
        let packet = self.queue.pop_front()?;

        let time_spent_queued = self
            .last_update
            .map_or(Duration::ZERO, |last_update| last_update - packet.queued_at);
        self.total_time_spent_queued = self
            .total_time_spent_queued
            .saturating_sub(time_spent_queued);
        self.packet_count -= 1;

        Some(packet)
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn update_average_queue_time(&mut self, now: Instant) {
        let Some(last_update) = self.last_update else {
            self.last_update = Some(now);
            return;
        };

        let elapsed = now - last_update;
        self.total_time_spent_queued += elapsed * self.packet_count;
        self.last_update = Some(now);
    }

    fn queue_state(&self, now: Instant) -> QueueState {
        QueueState {
            queue_id: self.queue_id,
            unpaced: self.is_audio,
            use_for_padding: !self.is_audio && self.last_emitted.is_some(),
            snapshot: QueueSnapshot {
                created_at: now,
                byte_size: self.queue.iter().map(QueuedPacket::size).sum(),
                packet_count: self.packet_count,
                total_queue_time_origin: self.total_time_spent_queued,
                last_emitted: self.last_emitted,
                first_unsent: self.queue.iter().next().map(|p| p.queued_at),
                priority: self.priority,
            },
        }
    }
}

use std::fmt;

impl fmt::Display for PacketKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacketKind::Audio => write!(f, "audio"),
            PacketKind::Video => write!(f, "video"),
            PacketKind::Padding => write!(f, "padding"),
        }
    }
}
