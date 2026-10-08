//! Voice frames held and released together, so several ride one message
//! (plan E4.3 T3): 3GPP's frame aggregation (TS 26.114 §10.2, Annex C), the
//! batch size chosen per route by
//! [`voice_frames_per_message`](crate::transport::allocation::voice_frames_per_message).
//!
//! The Opus encoder keeps its 20 ms frames, each still its own signed
//! packet with its own sequence numbers, so the receive path is unchanged:
//! N packets arrive together and the jitter buffer absorbs the burst. The
//! latency cost is the batch: the first frame of a batch of N waits
//! (N − 1) × 20 ms, the ptime cost RFC 7587 §5 describes.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::Queued;

/// One Opus frame.
const FRAME: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub(super) struct VoiceBatcher {
    hold: VecDeque<(Queued, usize, Instant)>,
    frames: usize,
}

impl Default for VoiceBatcher {
    fn default() -> Self {
        Self {
            hold: VecDeque::new(),
            frames: 1,
        }
    }
}

impl VoiceBatcher {
    /// Frames per batch from now on.
    pub(super) fn set_frames(&mut self, frames: usize) {
        self.frames = frames.max(1);
    }

    pub(super) fn frames(&self) -> usize {
        self.frames
    }

    /// Hold a voice datagram of `size` wire bytes that arrived at `now`.
    pub(super) fn push(&mut self, queued: Queued, size: usize, now: Instant) {
        self.hold.push_back((queued, size, now));
    }

    /// When the held batch is due: its first frame plus (N − 1) frames.
    pub(super) fn deadline(&self) -> Option<Instant> {
        let (_, _, first) = self.hold.front()?;
        Some(*first + FRAME * u32::try_from(self.frames - 1).unwrap_or(0))
    }

    /// The batch, when it is full or due; otherwise nothing.
    pub(super) fn release(&mut self, now: Instant) -> Vec<(Queued, usize)> {
        let due = self.deadline().is_some_and(|d| now >= d);
        if self.hold.len() >= self.frames || due {
            self.hold.drain(..).map(|(q, size, _)| (q, size)).collect()
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::media_frame::VOICE_TAG;

    fn voice() -> Queued {
        Queued {
            tag: VOICE_TAG,
            payload: Arc::from(vec![0u8; 100]),
            media_bytes: 100,
        }
    }

    #[test]
    fn one_frame_batches_release_at_once() {
        let mut b = VoiceBatcher::default();
        let t0 = Instant::now();
        b.push(voice(), 1_800, t0);
        assert_eq!(b.release(t0).len(), 1);
    }

    #[test]
    fn three_frame_batches_wait_for_three_or_their_deadline() {
        let mut b = VoiceBatcher::default();
        b.set_frames(3);
        let t0 = Instant::now();
        b.push(voice(), 1_800, t0);
        b.push(voice(), 1_800, t0 + FRAME);
        assert!(b.release(t0 + FRAME).is_empty());
        assert_eq!(b.deadline(), Some(t0 + FRAME * 2));
        b.push(voice(), 1_800, t0 + FRAME * 2);
        assert_eq!(b.release(t0 + FRAME * 2).len(), 3);
        // A batch cut short by silence goes at its deadline.
        b.push(voice(), 1_800, t0 + FRAME * 5);
        assert!(b.release(t0 + FRAME * 6).is_empty());
        assert_eq!(b.release(t0 + FRAME * 7).len(), 1);
    }
}
