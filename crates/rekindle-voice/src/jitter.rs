use crate::transport::VoicePacket;
use std::collections::BTreeMap;

/// Adaptive jitter buffer for smoothing out network timing variations.
///
/// Buffers incoming voice packets and releases them at a steady rate
/// to compensate for variable network latency. Includes an initial
/// buffering phase that waits until enough packets have accumulated
/// before allowing playback to start.
pub struct JitterBuffer {
    /// Buffered packets indexed by sequence number.
    buffer: BTreeMap<u32, VoicePacket>,
    /// Target buffer depth in milliseconds.
    target_delay_ms: u32,
    /// Next expected sequence number for playback.
    next_playback_seq: u32,
    /// Maximum number of packets to buffer before dropping old ones.
    max_packets: usize,
    /// Whether the initial buffering phase is complete.
    initial_fill_done: bool,
    /// Timestamp of the first packet arrival (for initial fill timing).
    first_packet_time: Option<std::time::Instant>,
}

impl JitterBuffer {
    /// Create a new jitter buffer with the given target delay.
    pub fn new(target_delay_ms: u32) -> Self {
        Self {
            buffer: BTreeMap::new(),
            target_delay_ms,
            next_playback_seq: 0,
            max_packets: 50,
            initial_fill_done: false,
            first_packet_time: None,
        }
    }

    /// Push an incoming packet into the buffer.
    pub fn push(&mut self, packet: VoicePacket) {
        let seq = packet.sequence;

        // Record first packet time for initial fill
        if self.first_packet_time.is_none() {
            self.first_packet_time = Some(std::time::Instant::now());
        }

        // Drop packets that are too old (already played)
        if self.initial_fill_done && seq < self.next_playback_seq {
            tracing::trace!(seq, expected = self.next_playback_seq, "dropping late packet");
            return;
        }

        self.buffer.insert(seq, packet);

        // Trim if buffer is too large
        while self.buffer.len() > self.max_packets {
            self.buffer.pop_first();
            if self.initial_fill_done {
                self.next_playback_seq += 1;
            }
        }
    }

    /// Pop the next packet for playback, if available.
    ///
    /// Returns `None` if the initial fill phase hasn't completed yet
    /// or the next expected packet hasn't arrived (packet loss / buffering).
    pub fn pop(&mut self) -> Option<VoicePacket> {
        // Don't start playback until initial fill is complete
        if !self.initial_fill_done {
            if !self.check_initial_fill() {
                return None;
            }
            // Set next_playback_seq to the first available sequence
            if let Some(&first_seq) = self.buffer.keys().next() {
                self.next_playback_seq = first_seq;
            }
        }

        let packet = self.buffer.remove(&self.next_playback_seq);
        if packet.is_some() {
            self.next_playback_seq += 1;
        }
        packet
    }

    /// Check if the initial fill phase should complete.
    ///
    /// Completes when either:
    /// - Enough time has elapsed since first packet (`target_delay_ms`)
    /// - Enough packets have accumulated (`target_delay_ms` / 20ms)
    fn check_initial_fill(&mut self) -> bool {
        let target_packets = (self.target_delay_ms / 20).max(1) as usize;

        if self.buffer.len() >= target_packets {
            self.initial_fill_done = true;
            return true;
        }

        if let Some(first_time) = self.first_packet_time {
            if first_time.elapsed().as_millis() >= u128::from(self.target_delay_ms) {
                self.initial_fill_done = true;
                return true;
            }
        }

        false
    }

    /// Peek at the next buffered packet after the expected one (for FEC recovery).
    ///
    /// When `pop()` returns `None` (current packet missing), this peeks at
    /// `next_playback_seq + 1` to check if FEC recovery is possible.
    /// Returns the audio data of the next packet if available.
    pub fn peek_next_audio_data(&self) -> Option<&[u8]> {
        if !self.initial_fill_done {
            return None;
        }
        let next_seq = self.next_playback_seq.wrapping_add(1);
        self.buffer.get(&next_seq).map(|p| p.audio_data.as_slice())
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
        self.initial_fill_done = false;
        self.first_packet_time = None;
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
        // Use 0ms target delay to skip initial fill (unit test only)
        let mut jb = JitterBuffer::new(0);
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
        let mut jb = JitterBuffer::new(0);
        jb.push(make_packet(2));
        jb.push(make_packet(0));
        jb.push(make_packet(1));

        assert_eq!(jb.pop().unwrap().sequence, 0);
        assert_eq!(jb.pop().unwrap().sequence, 1);
        assert_eq!(jb.pop().unwrap().sequence, 2);
    }

    #[test]
    fn test_late_packet_dropped() {
        let mut jb = JitterBuffer::new(0);
        jb.push(make_packet(0));
        jb.pop(); // consume 0, next_playback_seq = 1

        jb.push(make_packet(0)); // late, should be dropped
        assert_eq!(jb.depth(), 0);
    }
}
