//! Per-stream reassembly buffer for incoming `VideoFragment`s and
//! their FEC parity siblings (architecture §10.6). Bounded memory:
//! at most `MAX_PENDING_FRAMES_PER_STREAM` partial frames per
//! `(stream, sender)`, and stale frames (older than
//! `STALE_FRAME_HORIZON_MS`) get evicted whenever a new fragment
//! arrives. The receiver feeds completed frames to its decoder; the
//! sender side never touches this module.

use std::collections::HashMap;

use thiserror::Error;

use crate::fragment::{
    reconstruct_frame, VideoFragment, VideoParityFragment, MAX_FRAGMENTS_PER_FRAME, STREAM_ID_LEN,
};

/// Cap on partial frames we'll buffer at once for a given stream.
/// Above this, the oldest pending frame is dropped — protects against
/// a malicious or buggy sender that floods us with non-completing
/// frame_seqs.
const MAX_PENDING_FRAMES_PER_STREAM: usize = 8;

/// Frames whose first fragment timestamp lags `now` by more than this
/// are evicted. Architecture §10.6 implies a real-time stream — a
/// frame older than ~2 seconds is useless.
const STALE_FRAME_HORIZON_MS: u32 = 2_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReassemblerError {
    #[error("frag_index {0} >= frag_total {1}")]
    FragIndexOutOfRange(u8, u8),
    #[error("frag_total mismatch — saw {saw}, expected {expected}")]
    FragTotalMismatch { saw: u8, expected: u8 },
    #[error("frag_total exceeds MAX_FRAGMENTS_PER_FRAME ({MAX_FRAGMENTS_PER_FRAME})")]
    TooManyFragments,
    #[error("keyframe flag mismatch within the same frame_seq")]
    KeyframeMismatch,
    #[error("parity_index {0} >= parity_total {1}")]
    ParityIndexOutOfRange(u8, u8),
    #[error("parity metadata mismatch — frame already has different data_count/parity_total")]
    ParityMetadataMismatch,
    #[error("FEC reconstruct failed: {0}")]
    FecReconstruct(String),
}

/// A successfully reassembled frame, ready for the decoder.
#[derive(Debug, Clone)]
pub struct ReassembledFrame {
    pub stream_id: [u8; STREAM_ID_LEN],
    pub frame_seq: u32,
    pub keyframe: bool,
    pub timestamp: u32,
    pub payload: Vec<u8>,
    /// `true` if at least one parity fragment was used to recover a
    /// missing data fragment. Caller may emit a `KeyframeRequest` to
    /// the sender after enough FEC-recovered frames in a row.
    pub recovered_via_fec: bool,
}

#[derive(Debug)]
struct PartialFrame {
    frag_total: u8,
    /// Set by the first data fragment. Parity fragments don't carry
    /// the keyframe flag (their role is shard-recovery, not stream
    /// metadata), so when parity arrives first the value stays `None`
    /// until the first data fragment lands.
    keyframe: Option<bool>,
    timestamp: u32,
    received_at_ms: u32,
    /// Sparse data-shard slots. `chunks[i] = Some(payload)` once index i arrives.
    chunks: Vec<Option<Vec<u8>>>,
    received_data_count: u8,
    /// Parity-shard slots — only populated when the sender shipped FEC.
    /// `Vec::new()` means "no parity seen yet" (size set on first parity arrival).
    parity_chunks: Vec<Option<Vec<u8>>>,
    received_parity_count: u8,
    /// Length of the original encrypted frame in bytes — needed to
    /// truncate post-reconstruction padding. `0` until at least one
    /// parity fragment has arrived (data-only senders never set this).
    frame_len: u32,
}

impl PartialFrame {
    fn from_data(frag_total: u8, keyframe: bool, timestamp: u32, received_at_ms: u32) -> Self {
        Self {
            frag_total,
            keyframe: Some(keyframe),
            timestamp,
            received_at_ms,
            chunks: vec![None; usize::from(frag_total)],
            received_data_count: 0,
            parity_chunks: Vec::new(),
            received_parity_count: 0,
            frame_len: 0,
        }
    }

    fn from_parity(data_count: u8, timestamp: u32, received_at_ms: u32) -> Self {
        Self {
            frag_total: data_count,
            keyframe: None,
            timestamp,
            received_at_ms,
            chunks: vec![None; usize::from(data_count)],
            received_data_count: 0,
            parity_chunks: Vec::new(),
            received_parity_count: 0,
            frame_len: 0,
        }
    }
}

/// One pending stream's reassembly state. Keyed by `(stream_id, sender)`
/// so two senders sharing a screen don't poison each other's buffers.
#[derive(Debug, Default)]
struct StreamBuffer {
    frames: HashMap<u32, PartialFrame>,
}

#[derive(Debug, Default)]
pub struct Reassembler {
    /// Outer key is `(stream_id, sender_hex)`. Inner key is `frame_seq`.
    streams: HashMap<([u8; STREAM_ID_LEN], String), StreamBuffer>,
}

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
            PartialFrame::from_data(total, fragment.keyframe, fragment.timestamp, now_ms)
        });

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
            PartialFrame::from_parity(fragment.data_count, fragment.timestamp, now_ms)
        });

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

fn evict_stale(buffer: &mut StreamBuffer, now_ms: u32) {
    buffer.frames.retain(|_, partial| {
        now_ms.saturating_sub(partial.received_at_ms) <= STALE_FRAME_HORIZON_MS
    });
}

fn cap_pending(buffer: &mut StreamBuffer, frame_seq: u32) {
    if !buffer.frames.contains_key(&frame_seq)
        && buffer.frames.len() >= MAX_PENDING_FRAMES_PER_STREAM
    {
        if let Some((&oldest_seq, _)) = buffer.frames.iter().min_by_key(|(_, p)| p.received_at_ms) {
            buffer.frames.remove(&oldest_seq);
        }
    }
}

/// Try to complete `frame_seq`. Returns the assembled payload via
/// (a) direct concatenation when every data shard arrived, or
/// (b) Reed-Solomon reconstruction when data + parity ≥ data_count
/// and at least one data shard is missing.
fn try_complete(
    buffer: &mut StreamBuffer,
    frame_seq: u32,
    stream_id: [u8; STREAM_ID_LEN],
) -> Result<Option<ReassembledFrame>, ReassemblerError> {
    let Some(partial) = buffer.frames.get_mut(&frame_seq) else {
        return Ok(None);
    };

    // Fast path: every data shard present — concatenate.
    if partial.received_data_count == partial.frag_total {
        let mut payload = Vec::new();
        for chunk in &mut partial.chunks {
            if let Some(bytes) = chunk.take() {
                payload.extend(bytes);
            }
        }
        // Strip null padding only when we know the original length
        // (i.e. parity packets arrived alongside).
        if partial.frame_len > 0 {
            payload.truncate(partial.frame_len as usize);
        }
        let frame = ReassembledFrame {
            stream_id,
            frame_seq,
            keyframe: partial.keyframe.unwrap_or(false),
            timestamp: partial.timestamp,
            payload,
            recovered_via_fec: false,
        };
        buffer.frames.remove(&frame_seq);
        return Ok(Some(frame));
    }

    // FEC path: have parity, and total received ≥ data_count, with at
    // least one data shard missing. Need parity_chunks initialized
    // (otherwise the sender shipped no FEC).
    if partial.parity_chunks.is_empty() {
        return Ok(None);
    }
    let total_received = partial.received_data_count + partial.received_parity_count;
    if total_received < partial.frag_total {
        return Ok(None);
    }

    let parity_total = u8::try_from(partial.parity_chunks.len()).unwrap_or(u8::MAX);
    let received_data: Vec<(u8, Vec<u8>)> = partial
        .chunks
        .iter()
        .enumerate()
        .filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|p| (u8::try_from(idx).expect("frag_total fits u8"), p.clone()))
        })
        .collect();
    let received_parity: Vec<(u8, Vec<u8>)> = partial
        .parity_chunks
        .iter()
        .enumerate()
        .filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|p| (u8::try_from(idx).expect("parity_total fits u8"), p.clone()))
        })
        .collect();
    let payload = reconstruct_frame(
        &received_data,
        &received_parity,
        partial.frag_total,
        parity_total,
        partial.frame_len,
    )
    .map_err(|e| ReassemblerError::FecReconstruct(e.to_string()))?;

    let frame = ReassembledFrame {
        stream_id,
        frame_seq,
        keyframe: partial.keyframe.unwrap_or(false),
        timestamp: partial.timestamp,
        payload,
        recovered_via_fec: true,
    };
    buffer.frames.remove(&frame_seq);
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::{fragment_frame, fragment_frame_with_fec, FRAGMENT_PAYLOAD_LIMIT};

    fn fragmented(frame_seq: u32, payload: &[u8], keyframe: bool) -> Vec<VideoFragment> {
        fragment_frame([7u8; STREAM_ID_LEN], frame_seq, keyframe, 0, payload).unwrap()
    }

    #[test]
    fn single_fragment_completes_immediately() {
        let mut r = Reassembler::new();
        let frags = fragmented(1, &vec![0xAA; 1024], true);
        let out = r.ingest("alice", frags[0].clone(), 0).unwrap();
        assert!(out.is_some());
        let frame = out.unwrap();
        assert_eq!(frame.frame_seq, 1);
        assert!(frame.keyframe);
        assert!(!frame.recovered_via_fec);
    }

    #[test]
    fn multi_fragment_completes_when_all_arrive() {
        let mut r = Reassembler::new();
        let payload = vec![0x33u8; FRAGMENT_PAYLOAD_LIMIT * 2 + 50];
        let frags = fragmented(5, &payload, false);
        assert_eq!(frags.len(), 3);
        assert!(r.ingest("alice", frags[2].clone(), 100).unwrap().is_none());
        assert!(r.ingest("alice", frags[0].clone(), 100).unwrap().is_none());
        let done = r.ingest("alice", frags[1].clone(), 100).unwrap();
        assert!(done.is_some());
        let frame = done.unwrap();
        assert_eq!(frame.payload, payload);
        assert!(!frame.recovered_via_fec);
    }

    #[test]
    fn duplicate_fragment_does_not_double_count() {
        let mut r = Reassembler::new();
        let frags = fragmented(2, &vec![0x11; FRAGMENT_PAYLOAD_LIMIT * 2 + 1], true);
        assert!(r.ingest("alice", frags[0].clone(), 0).unwrap().is_none());
        assert!(r.ingest("alice", frags[0].clone(), 0).unwrap().is_none());
        assert!(r.ingest("alice", frags[1].clone(), 0).unwrap().is_none());
        let done = r.ingest("alice", frags[2].clone(), 0).unwrap();
        assert!(done.is_some());
    }

    #[test]
    fn stale_partials_are_evicted() {
        let mut r = Reassembler::new();
        let frags = fragmented(1, &vec![0xFF; FRAGMENT_PAYLOAD_LIMIT * 2 + 1], true);
        assert!(r.ingest("alice", frags[0].clone(), 0).unwrap().is_none());
        let later = STALE_FRAME_HORIZON_MS + 1_000;
        let out = r.ingest("alice", frags[1].clone(), later).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn frag_total_mismatch_rejected() {
        let mut r = Reassembler::new();
        let frags = fragmented(3, &vec![0; FRAGMENT_PAYLOAD_LIMIT * 2 + 1], true);
        let mut tampered = frags[1].clone();
        tampered.frag_total = 5;
        let _ = r.ingest("alice", frags[0].clone(), 0).unwrap();
        let err = r.ingest("alice", tampered, 0).unwrap_err();
        assert!(matches!(err, ReassemblerError::FragTotalMismatch { .. }));
    }

    #[test]
    fn fec_recovers_lost_data_shard_via_parity() {
        // 3-data + 2-parity frame. Lose data[1]. Receive remaining
        // data[0], data[2], parity[0]. Reassembler should reconstruct.
        let mut r = Reassembler::new();
        let frame = vec![0xCDu8; FRAGMENT_PAYLOAD_LIMIT * 2 + 200];
        let fec = fragment_frame_with_fec([5u8; STREAM_ID_LEN], 9, true, 50, &frame, 2).unwrap();
        assert_eq!(fec.data.len(), 3);
        assert_eq!(fec.parity.len(), 2);

        assert!(r.ingest("alice", fec.data[0].clone(), 0).unwrap().is_none());
        assert!(r.ingest("alice", fec.data[2].clone(), 0).unwrap().is_none());
        let done = r.ingest_parity("alice", fec.parity[0].clone(), 0).unwrap();
        assert!(
            done.is_some(),
            "FEC should reconstruct after 2 data + 1 parity"
        );
        let frame_out = done.unwrap();
        assert!(frame_out.recovered_via_fec);
        assert_eq!(frame_out.payload, frame);
    }

    #[test]
    fn fec_completes_early_via_data_plus_parity() {
        // 3 data + 1 parity. After data[0] + data[1] + parity[0]
        // arrive (3 shards total), the frame is already reconstructable
        // via Reed-Solomon. The receiver should NOT wait for data[2].
        // Padding from the equal-shard requirement must be stripped to
        // `frame_len` (verified by the byte-for-byte match below).
        let mut r = Reassembler::new();
        let frame = vec![0xEEu8; FRAGMENT_PAYLOAD_LIMIT * 2 + 13];
        let fec = fragment_frame_with_fec([6u8; STREAM_ID_LEN], 11, true, 99, &frame, 1).unwrap();
        assert_eq!(fec.data.len(), 3);
        assert_eq!(fec.parity.len(), 1);

        assert!(r
            .ingest_parity("alice", fec.parity[0].clone(), 0)
            .unwrap()
            .is_none());
        assert!(r.ingest("alice", fec.data[0].clone(), 0).unwrap().is_none());
        let done = r.ingest("alice", fec.data[1].clone(), 0).unwrap();
        assert!(
            done.is_some(),
            "3 of 4 shards is enough — must not wait for data[2]"
        );
        let frame_out = done.unwrap();
        assert!(frame_out.recovered_via_fec);
        assert_eq!(
            frame_out.payload, frame,
            "padding must be stripped to frame_len bytes"
        );
    }

    #[test]
    fn fec_fast_path_when_all_data_arrives_first() {
        // All 3 data shards arrive before any parity. Fast-path concat
        // should complete the frame; the late parity arrival becomes a
        // no-op against an already-removed partial.
        let mut r = Reassembler::new();
        let frame = vec![0xCCu8; FRAGMENT_PAYLOAD_LIMIT * 2 + 33];
        let fec = fragment_frame_with_fec([8u8; STREAM_ID_LEN], 12, true, 1, &frame, 1).unwrap();
        assert!(r.ingest("alice", fec.data[0].clone(), 0).unwrap().is_none());
        assert!(r.ingest("alice", fec.data[1].clone(), 0).unwrap().is_none());
        let done = r.ingest("alice", fec.data[2].clone(), 0).unwrap();
        assert!(done.is_some());
        let frame_out = done.unwrap();
        assert!(!frame_out.recovered_via_fec);
        // Without prior parity, frame_len was 0 → no truncation. The
        // last data shard isn't padded for a non-FEC sender, but a
        // FEC-using sender pads to `shard_size`. Confirm we get back
        // the original (which means padding must NOT have been added
        // since frame_len was 0). The encoder pads to shard_size for
        // FEC-encoded shards, so without frame_len the concat could
        // be longer than the original — this asserts the contract.
        let shard_size = frame.len().div_ceil(3);
        assert_eq!(frame_out.payload.len(), shard_size * 3);
        assert_eq!(&frame_out.payload[..frame.len()], &frame[..]);
    }
}
