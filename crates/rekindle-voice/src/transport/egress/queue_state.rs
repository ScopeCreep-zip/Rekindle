//! The queue states the pacer chooses from.

use std::time::Instant;

use rekindle_media_bwe::{QueuePriority, QueueSnapshot, QueueState};

use super::{RouteController, MAX_PADDING_BYTES, UNPACED_QUEUE, VIDEO_QUEUE};

impl RouteController {
    pub(super) fn unpaced_state(&mut self, now: Instant) -> QueueState {
        QueueState {
            queue_id: UNPACED_QUEUE,
            unpaced: true,
            use_for_padding: false,
            snapshot: self.unpaced.snapshot(now),
        }
    }

    /// The video queue's state with the outstanding padding merged in, as
    /// str0m merges a stream's padding into its queue state
    /// (`queue_state_padding`, blank-padding form).
    pub(super) fn video_state(&mut self, now: Instant) -> QueueState {
        // Video due just before a voice batch waits for it, to share its
        // message (plan E4.3 T3).
        if self.video_waits_for_voice(now) {
            return QueueState {
                queue_id: VIDEO_QUEUE,
                unpaced: false,
                use_for_padding: true,
                snapshot: QueueSnapshot::default(),
            };
        }
        let mut snapshot = self.video.snapshot(now);
        if self.padding > 0 {
            snapshot.merge(&QueueSnapshot {
                created_at: now,
                byte_size: self.padding,
                packet_count: u32::try_from(self.padding.div_ceil(MAX_PADDING_BYTES))
                    .unwrap_or(u32::MAX),
                first_unsent: Some(now),
                priority: QueuePriority::Padding,
                ..QueueSnapshot::default()
            });
        }
        QueueState {
            queue_id: VIDEO_QUEUE,
            unpaced: false,
            use_for_padding: true,
            snapshot,
        }
    }
}
