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
    /// Packets discarded by the over-capacity trim since the last
    /// `take_drops` — the receive-side health signal the quality event
    /// surfaces (Phase 5).
    overflow_drops: u64,
    /// Packets discarded for arriving after their playback slot.
    late_drops: u64,
    /// Consecutive `pop()` misses at the current playback position —
    /// drives the gap jump in [`Self::note_miss_and_maybe_jump`].
    gap_misses: u32,
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
            overflow_drops: 0,
            late_drops: 0,
            gap_misses: 0,
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
            self.late_drops += 1;
            tracing::trace!(
                seq,
                expected = self.next_playback_seq,
                "dropping late packet"
            );
            return;
        }

        self.buffer.insert(seq, packet);

        // Trim if buffer is too large
        while self.buffer.len() > self.max_packets {
            self.buffer.pop_first();
            self.overflow_drops += 1;
            if self.initial_fill_done {
                self.next_playback_seq += 1;
            }
        }
    }

    /// Drain the drop counters (overflow, late) accumulated since the
    /// last call — read on the 5 s quality cadence.
    pub fn take_drops(&mut self) -> (u64, u64) {
        let drops = (self.overflow_drops, self.late_drops);
        self.overflow_drops = 0;
        self.late_drops = 0;
        drops
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
            self.gap_misses = 0;
        }
        packet
    }

    /// One playout tick failed to find the expected packet. After a
    /// full jitter window of consecutive misses (the packet is lost,
    /// not late), jump the gap: resume from the oldest buffered packet
    /// — the voice analog of the video playout buffer's keyframe jump.
    /// Without this a single lost packet stalls the stream forever:
    /// playback stays pinned at the missing seq while the overflow
    /// trim discards one GOOD packet per arrival (observed live as
    /// rx_overflow_drops in the hundreds per 5 s with dead audio until
    /// the participant timed out and re-seeded).
    pub fn note_miss_and_maybe_jump(&mut self) -> Option<VoicePacket> {
        if !self.initial_fill_done {
            return None;
        }
        self.gap_misses += 1;
        let window_ticks = (self.target_delay_ms / 20).max(1);
        if self.gap_misses < window_ticks || self.buffer.is_empty() {
            return None;
        }
        let (&seq, _) = self.buffer.iter().next()?;
        self.gap_misses = 0;
        self.next_playback_seq = seq.wrapping_add(1);
        self.buffer.remove(&seq)
    }

    /// FEC reconstructed the frame at the current playback position —
    /// advance past it so the next pop returns the real buffered packet
    /// the FEC data was peeked from, instead of re-concealing the same
    /// position every tick.
    pub fn advance_after_fec(&mut self) {
        self.next_playback_seq = self.next_playback_seq.wrapping_add(1);
        self.gap_misses = 0;
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
        self.gap_misses = 0;
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
            mek_generation: 0,
            signature: Vec::new(),
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
        assert_eq!(jb.take_drops(), (0, 1), "late drop counted");
        assert_eq!(jb.take_drops(), (0, 0), "take_drops resets");
    }

    #[test]
    fn gap_jump_resumes_from_oldest_buffered() {
        // target_delay 0 → jump window = 1 tick: the first noted miss
        // already exceeds the jitter window.
        let mut jb = JitterBuffer::new(0);
        jb.push(make_packet(0));
        jb.push(make_packet(1));
        // seq 2 lost; 3 and 4 arrive.
        jb.push(make_packet(3));
        jb.push(make_packet(4));
        assert_eq!(jb.pop().unwrap().sequence, 0);
        assert_eq!(jb.pop().unwrap().sequence, 1);
        assert!(jb.pop().is_none(), "seq 2 is lost");
        let jumped = jb.note_miss_and_maybe_jump().unwrap();
        assert_eq!(jumped.sequence, 3, "resumes from oldest buffered");
        assert_eq!(jb.pop().unwrap().sequence, 4, "stream continues in order");
    }

    #[test]
    fn gap_jump_waits_out_the_jitter_window() {
        // 60ms target → 3 ticks of grace before jumping.
        let mut jb = JitterBuffer::new(60);
        for seq in 0..3 {
            jb.push(make_packet(seq));
        }
        while jb.pop().is_some() {}
        jb.push(make_packet(5)); // seq 3 + 4 lost
        assert!(jb.note_miss_and_maybe_jump().is_none(), "miss 1: wait");
        assert!(jb.note_miss_and_maybe_jump().is_none(), "miss 2: wait");
        let jumped = jb.note_miss_and_maybe_jump().unwrap();
        assert_eq!(jumped.sequence, 5, "miss 3: jump");
    }

    #[test]
    fn fec_advance_unsticks_playback_position() {
        let mut jb = JitterBuffer::new(0);
        jb.push(make_packet(0));
        assert_eq!(jb.pop().unwrap().sequence, 0);
        // seq 1 lost, 2 buffered → FEC peek sees 2's payload.
        jb.push(make_packet(2));
        assert!(jb.pop().is_none());
        assert!(jb.peek_next_audio_data().is_some());
        jb.advance_after_fec();
        assert_eq!(
            jb.pop().unwrap().sequence,
            2,
            "next pop returns the packet FEC peeked, not another conceal"
        );
    }

    #[test]
    fn lost_packet_no_longer_stalls_into_endless_overflow() {
        // Regression for the live failure: one lost packet pinned
        // playback while every later arrival was trimmed as overflow
        // (rx_overflow_drops in the hundreds per 5 s, dead audio).
        let mut jb = JitterBuffer::new(0);
        jb.push(make_packet(0));
        assert_eq!(jb.pop().unwrap().sequence, 0);
        // seq 1 lost; a long run of later packets arrives.
        for seq in 2..80 {
            jb.push(make_packet(seq));
        }
        // Playout tick: miss → jump → stream drains in order.
        assert!(jb.pop().is_none());
        assert!(jb.note_miss_and_maybe_jump().is_some());
        let mut drained = 1;
        while jb.pop().is_some() {
            drained += 1;
        }
        let (overflow, _) = jb.take_drops();
        // 78 pushed after the gap; max_packets=50 bounds the buffer, so
        // pre-jump trims are expected — but everything still buffered
        // plays out instead of being discarded one-per-arrival forever.
        assert_eq!(drained + overflow, 78, "every packet played or trimmed once");
        assert!(drained >= 50, "the surviving window drains fully");
    }

    #[test]
    fn test_overflow_trim_counted() {
        let mut jb = JitterBuffer::new(0);
        for seq in 0..60 {
            jb.push(make_packet(seq));
        }
        // max_packets = 50 → ten packets trimmed.
        assert_eq!(jb.depth(), 50);
        let (overflow, late) = jb.take_drops();
        assert_eq!(overflow, 10, "overflow trim counted");
        assert_eq!(late, 0);
    }
}
