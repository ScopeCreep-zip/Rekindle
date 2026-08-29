//! Fragment/parity ingestion — the mutable entry points of the
//! reassembler (see `mod.rs` for the types and eviction policy).

use super::complete::{cap_pending, evict_stale, try_complete};
use super::*;

impl Reassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest a data fragment, return a fully-reassembled frame when
    /// ready (either all data shards present, or enough data+parity
    /// to FEC-reconstruct). `now_ms` is the receiver's current
    /// wall-clock; older frames are evicted relative to it. Caller has
    /// already verified the fragment's signature against the sender's
    /// pseudonym key.
    pub fn ingest(
        &mut self,
        sender_hex: &str,
        fragment: VideoFragment,
        now_ms: u32,
    ) -> Result<Option<ReassembledFrame>, ReassemblerError> {
        let total = fragment.frag_total;
        if usize::from(total) > MAX_FRAGMENTS_PER_FRAME {
            return Err(ReassemblerError::TooManyFragments);
        }
        if total == 0 || fragment.frag_index >= total {
            return Err(ReassemblerError::FragIndexOutOfRange(
                fragment.frag_index,
                total,
            ));
        }

        let key = (fragment.stream_id, sender_hex.to_string());
        let buffer = self.streams.entry(key).or_default();
        evict_stale(buffer, now_ms);
        cap_pending(buffer, fragment.frame_seq);

        let partial = buffer.frames.entry(fragment.frame_seq).or_insert_with(|| {
            PartialFrame::from_data(
                total,
                fragment.keyframe,
                fragment.codec,
                fragment.timestamp,
                now_ms,
                fragment.mek_generation,
            )
        });

        if partial.codec != fragment.codec {
            return Err(ReassemblerError::CodecMismatch);
        }
        if partial.mek_generation != fragment.mek_generation {
            return Err(ReassemblerError::MekGenerationMismatch);
        }
        if partial.frag_total != total {
            return Err(ReassemblerError::FragTotalMismatch {
                saw: total,
                expected: partial.frag_total,
            });
        }
        match partial.keyframe {
            Some(existing) if existing != fragment.keyframe => {
                return Err(ReassemblerError::KeyframeMismatch);
            }
            None => {
                // First data fragment fills in the keyframe flag — parity-
                // arrived-first path leaves it unset.
                partial.keyframe = Some(fragment.keyframe);
            }
            Some(_) => {}
        }

        let slot = &mut partial.chunks[usize::from(fragment.frag_index)];
        if slot.is_none() {
            *slot = Some(fragment.payload);
            partial.received_data_count = partial.received_data_count.saturating_add(1);
            // Last-ACTIVITY eviction anchor: a large paced frame's
            // fragments legitimately span more than the stale horizon
            // at low rates — anchoring on the FIRST fragment evicted
            // partials right before their completing shard arrived,
            // making big keyframes structurally uncompletable.
            partial.received_at_ms = now_ms;
        }

        try_complete(buffer, fragment.frame_seq, fragment.stream_id)
    }

    /// Ingest a parity (FEC) fragment. Same return contract as
    /// `ingest`: completes the frame when enough total shards have
    /// arrived to either concat the data path or reconstruct via
    /// Reed-Solomon.
    pub fn ingest_parity(
        &mut self,
        sender_hex: &str,
        fragment: VideoParityFragment,
        now_ms: u32,
    ) -> Result<Option<ReassembledFrame>, ReassemblerError> {
        if fragment.data_count == 0
            || fragment.parity_total == 0
            || fragment.parity_index >= fragment.parity_total
        {
            return Err(ReassemblerError::ParityIndexOutOfRange(
                fragment.parity_index,
                fragment.parity_total,
            ));
        }
        if usize::from(fragment.data_count) + usize::from(fragment.parity_total)
            > MAX_FRAGMENTS_PER_FRAME
        {
            return Err(ReassemblerError::TooManyFragments);
        }

        let key = (fragment.stream_id, sender_hex.to_string());
        let buffer = self.streams.entry(key).or_default();
        evict_stale(buffer, now_ms);
        cap_pending(buffer, fragment.frame_seq);

        let partial = buffer.frames.entry(fragment.frame_seq).or_insert_with(|| {
            PartialFrame::from_parity(
                fragment.data_count,
                fragment.codec,
                fragment.timestamp,
                now_ms,
                fragment.mek_generation,
            )
        });

        if partial.codec != fragment.codec {
            return Err(ReassemblerError::CodecMismatch);
        }
        if partial.frag_total != fragment.data_count {
            return Err(ReassemblerError::ParityMetadataMismatch);
        }
        if partial.parity_chunks.is_empty() {
            partial.parity_chunks = vec![None; usize::from(fragment.parity_total)];
        } else if partial.parity_chunks.len() != usize::from(fragment.parity_total) {
            return Err(ReassemblerError::ParityMetadataMismatch);
        }
        if partial.frame_len == 0 {
            partial.frame_len = fragment.frame_len;
        } else if partial.frame_len != fragment.frame_len {
            return Err(ReassemblerError::ParityMetadataMismatch);
        }

        let slot = &mut partial.parity_chunks[usize::from(fragment.parity_index)];
        if slot.is_none() {
            *slot = Some(fragment.payload);
            partial.received_parity_count = partial.received_parity_count.saturating_add(1);
            // Same last-activity anchor as the data path.
            partial.received_at_ms = now_ms;
        }

        try_complete(buffer, fragment.frame_seq, fragment.stream_id)
    }

    /// Drop every pending frame from `(stream_id, sender)`. Called when
    /// a `KeyframeRequest` is sent so we don't accumulate stale partials
    /// while waiting for the next I-frame.
    pub fn reset_stream(&mut self, stream_id: [u8; STREAM_ID_LEN], sender_hex: &str) {
        self.streams.remove(&(stream_id, sender_hex.to_string()));
    }
}
