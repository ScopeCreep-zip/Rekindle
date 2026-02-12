use crate::transport::VoicePacket;
use std::collections::BTreeMap;

/// Adaptive jitter buffer for smoothing out network timing variations.
///
/// Buffers incoming voice packets and releases them at a steady rate
/// to compensate for variable network latency.
pub struct JitterBuffer {
    /// Buffered packets indexed by sequence number.
    buffer: BTreeMap<u32, VoicePacket>,
    /// Target buffer depth in milliseconds.
    target_delay_ms: u32,
    /// Next expected sequence number for playback.
    next_playback_seq: u32,
    /// Maximum number of packets to buffer before dropping old ones.
    max_packets: usize,
}

impl JitterBuffer {
    /// Create a new jitter buffer with the given target delay.
    pub fn new(target_delay_ms: u32) -> Self {
        Self {
            buffer: BTreeMap::new(),
            target_delay_ms,
            next_playback_seq: 0,
            max_packets: 50,
        }
    }

    /// Push an incoming packet into the buffer.
    pub fn push(&mut self, packet: VoicePacket) {
        let seq = packet.sequence;

        // Drop packets that are too old (already played)
        if seq < self.next_playback_seq {
            tracing::trace!(seq, expected = self.next_playback_seq, "dropping late packet");
            return;
        }

        self.buffer.insert(seq, packet);

        // Trim if buffer is too large
        while self.buffer.len() > self.max_packets {
            self.buffer.pop_first();
            self.next_playback_seq += 1;
        }
    }

    /// Pop the next packet for playback, if available.
    ///
    /// Returns `None` if the next expected packet hasn't arrived yet
    /// (packet loss or still buffering).
    pub fn pop(&mut self) -> Option<VoicePacket> {
        let packet = self.buffer.remove(&self.next_playback_seq);
        if packet.is_some() {
            self.next_playback_seq += 1;
        }
        packet
    }

    /// Get the current buffer depth (number of buffered packets).
    pub fn depth(&self) -> usize {
        self.buffer.len()
    }

    /// Get the target delay in milliseconds.
    pub fn target_delay_ms(&self) -> u32 {
        self.target_delay_ms
    }

    /// Set a new target delay.
    pub fn set_target_delay_ms(&mut self, ms: u32) {
        self.target_delay_ms = ms;
    }

    /// Reset the buffer (e.g., on reconnect).
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.next_playback_seq = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_packet(seq: u32) -> VoicePacket {
        VoicePacket {
            sender_key: vec![0; 32],
            sequence: seq,
            timestamp: u64::from(seq) * 20,
            audio_data: vec![0; 160],
        }
    }

    #[test]
    fn test_in_order_playback() {
        let mut jb = JitterBuffer::new(60);
        jb.push(make_packet(0));
        jb.push(make_packet(1));
        jb.push(make_packet(2));

        assert_eq!(jb.pop().unwrap().sequence, 0);
        assert_eq!(jb.pop().unwrap().sequence, 1);
        assert_eq!(jb.pop().unwrap().sequence, 2);
        assert!(jb.pop().is_none());
    }

    #[test]
    fn test_out_of_order() {
        let mut jb = JitterBuffer::new(60);
        jb.push(make_packet(2));
        jb.push(make_packet(0));
        jb.push(make_packet(1));

        assert_eq!(jb.pop().unwrap().sequence, 0);
        assert_eq!(jb.pop().unwrap().sequence, 1);
        assert_eq!(jb.pop().unwrap().sequence, 2);
    }

    #[test]
    fn test_late_packet_dropped() {
        let mut jb = JitterBuffer::new(60);
        jb.push(make_packet(0));
        jb.pop(); // consume 0, next_playback_seq = 1

        jb.push(make_packet(0)); // late, should be dropped
        assert_eq!(jb.depth(), 0);
    }
}
