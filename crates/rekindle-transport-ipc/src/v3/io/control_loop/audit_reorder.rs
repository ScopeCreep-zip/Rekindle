//! Audit chain reorder buffer — advances the chain in strict session_seq
//! order regardless of rayon worker completion order.
//!
//! One instance per direction (outbound, inbound). Backed by a ReorderRing
//! with a power-of-two window providing a bounded reorder capacity. Zero
//! per-item allocation in steady state.
//!
//! Fast path via try_deliver_direct: when session_seq == next_expected
//! and the ring is empty, the entry is returned directly without touching
//! the slot array. This is the common case for inline-encoded frames and
//! single-stream bulk transfers where rayon workers complete in submission
//! order.

use std::time::Instant;

use rekindle_transport_buff::ReorderRing;

use crate::v3::audit::chain::LinkInput;

/// Default reorder window. Power of two >= the stall threshold.
/// Provides an actual windowed bound on out-of-order entries.
const DEFAULT_WINDOW: usize = 8192;

/// Maximum stored entries before declaring a stall.
/// If a rayon worker panics and never sends its LinkInput, the ring
/// fills with entries beyond the missing seq. When stored_count
/// exceeds this limit, the session should be terminated.
const MAX_STORED_BEFORE_STALL: usize = 4096;

/// Sliding-window reorder buffer for audit chain LinkInputs.
pub(crate) struct AuditReorderBuffer {
    ring: ReorderRing<LinkInput>,
    /// Timestamp when next_expected last advanced. Used for stall detection.
    last_advance: Instant,
}

impl AuditReorderBuffer {
    pub fn new() -> Self {
        Self {
            ring: ReorderRing::new(DEFAULT_WINDOW),
            last_advance: Instant::now(),
        }
    }

    /// Insert a LinkInput and drain all contiguous entries through `f`.
    ///
    /// Replaces the prior insert() + while flush_next() pattern with a
    /// single call that delivers the entire contiguous prefix per invocation.
    pub fn insert_and_drain(
        &mut self,
        link: LinkInput,
        mut f: impl FnMut(LinkInput),
    ) {
        let seq = link.session_seq;

        // Fast path: in-order, nothing buffered — deliver directly.
        match self.ring.try_deliver_direct(seq, link) {
            Ok(link) => {
                self.last_advance = Instant::now();
                f(link);
            }
            Err(link) => {
                // Out-of-order or items buffered — publish to ring.
                let _ = self.ring.publish(seq, link);
            }
        }

        // Drain the contiguous prefix. Delivers zero items if a gap
        // exists at next_deliver. Delivers the whole prefix otherwise.
        self.ring.drain_contiguous(|_seq, entry| {
            self.last_advance = Instant::now();
            f(entry);
        });
    }

    /// The next session_seq this buffer expects to flush.
    pub fn next_expected(&self) -> u64 {
        self.ring.next_deliver()
    }

    /// Number of out-of-order entries currently stored in the ring.
    pub fn buffered_count(&self) -> usize {
        self.ring.stored_count()
    }

    /// Returns true if the buffer appears stalled — entries are accumulating
    /// but next_expected hasn't advanced. Indicates a rayon worker panicked
    /// and never sent its LinkInput for session_seq == next_expected.
    pub fn is_stalled(&self) -> bool {
        self.ring.stored_count() > MAX_STORED_BEFORE_STALL
    }

    /// Duration since next_expected last advanced. For diagnostics.
    pub fn time_since_last_advance(&self) -> std::time::Duration {
        self.last_advance.elapsed()
    }
}
