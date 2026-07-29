//! Audit chain reorder buffer — advances the chain in strict session_seq
//! order regardless of rayon worker completion order.
//!
//! One instance per direction (outbound, inbound). Backed by a ReorderRing
//! with a power-of-two window providing a bounded reorder capacity. Zero
//! per-item allocation in steady state.
//!
//! Stall detection uses time + buffered count: the buffer is stalled when
//! entries are buffered AND time_since_last_advance exceeds a configurable
//! threshold. Rayon completion reordering resolves in milliseconds — a stall
//! lasting hundreds of milliseconds means a frame is genuinely lost.
//!
//! Fast path via try_deliver_direct: when session_seq == next_expected
//! and the ring is empty, the entry is returned directly without touching
//! the slot array.

use std::time::{Duration, Instant};

use rekindle_transport_buff::ReorderRing;

use crate::v4::audit::chain::LinkInput;

/// Default reorder window. Power of two >= expected max out-of-order distance.
const DEFAULT_WINDOW: usize = 8192;

/// Sliding-window reorder buffer for audit chain LinkInputs.
pub(crate) struct AuditReorderBuffer {
    ring: ReorderRing<LinkInput>,
    /// Timestamp when next_expected last advanced.
    last_advance: Instant,
    /// Maximum duration without advancement before declaring a stall.
    max_stall_duration: Duration,
    /// Label for tracing.
    label: &'static str,
}

impl AuditReorderBuffer {
    pub fn with_label(label: &'static str, max_stall_duration: Duration) -> Self {
        Self {
            ring: ReorderRing::new(DEFAULT_WINDOW),
            last_advance: Instant::now(),
            max_stall_duration,
            label,
        }
    }

    /// Insert a LinkInput and drain all contiguous entries through `f`.
    pub fn insert_and_drain(
        &mut self,
        link: LinkInput,
        mut f: impl FnMut(LinkInput),
    ) {
        let seq = link.session_seq;

        match self.ring.try_deliver_direct(seq, link) {
            Ok(link) => {
                tracing::trace!(
                    seq,
                    label = self.label,
                    "audit_reorder: fast path — delivered directly"
                );
                self.last_advance = Instant::now();
                f(link);
                return;
            }
            Err(link) => {
                let publish_result = self.ring.publish(seq, link);
                tracing::trace!(
                    seq,
                    publish_ok = publish_result.is_ok(),
                    label = self.label,
                    "audit_reorder: out-of-order — published to ring"
                );
            }
        }

        let mut drained = 0u64;
        self.ring.drain_contiguous(|_seq, entry| {
            drained += 1;
            f(entry);
        });
        if drained > 0 {
            self.last_advance = Instant::now();
            tracing::trace!(
                drained,
                next_deliver_after = self.ring.next_deliver(),
                stored_after = self.ring.stored_count(),
                label = self.label,
                "audit_reorder: drain_contiguous complete"
            );
        }
    }

    /// The next session_seq this buffer expects to flush.
    pub fn next_expected(&self) -> u64 {
        self.ring.next_deliver()
    }

    /// Number of out-of-order entries currently stored in the ring.
    pub fn buffered_count(&self) -> usize {
        self.ring.stored_count()
    }

    /// Returns true if the buffer is stalled: entries are buffered AND
    /// no advancement for longer than max_stall_duration. Rayon reordering
    /// resolves in milliseconds — a stall exceeding the threshold means
    /// a frame is genuinely lost, not temporarily late.
    pub fn is_stalled(&self) -> bool {
        self.ring.stored_count() > 0
            && self.last_advance.elapsed() > self.max_stall_duration
    }

    /// The contiguous range of missing seqs at the head of the buffer.
    /// Returns None if no items are buffered or next_expected is present.
    /// Returns Some((gap_start, gap_end)) inclusive — every seq in the
    /// range is missing from the ring.
    pub fn gap_range(&self) -> Option<(u64, u64)> {
        self.ring.gap_range()
    }

    /// Duration since next_expected last advanced.
    pub fn time_since_last_advance(&self) -> Duration {
        self.last_advance.elapsed()
    }
}
