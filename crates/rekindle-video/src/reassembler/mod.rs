//! Per-stream reassembly buffer for incoming `VideoFragment`s and
//! their FEC parity siblings (architecture §10.6). Bounded memory:
//! at most `MAX_PENDING_FRAMES_PER_STREAM` partial frames per
//! `(stream, sender)`, and stale frames (older than
//! `STALE_FRAME_HORIZON_MS`) get evicted whenever a new fragment
//! arrives. The receiver feeds completed frames to its decoder; the
//! sender side never touches this module.

use std::collections::HashMap;

use rekindle_types::video::Codec;

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
    #[error("codec tag mismatch within the same frame_seq")]
    CodecMismatch,
    #[error("MEK generation mismatch within the same frame_seq")]
    MekGenerationMismatch,
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
    /// Codec tag carried by the frame's fragments — forwarded on
    /// `VideoEvent::FrameReady` so the frontend configures the right
    /// decoder.
    pub codec: Codec,
    pub timestamp: u32,
    /// Generation of the channel-media MEK that encrypted the frame —
    /// from the fragments; the decrypt step requests exactly this
    /// generation on failure.
    pub mek_generation: u64,
    pub payload: Vec<u8>,
    /// `true` if at least one parity fragment was used to recover a
    /// missing data fragment. Caller may emit a `KeyframeRequest` to
    /// the sender after enough FEC-recovered frames in a row.
    pub recovered_via_fec: bool,
}

#[derive(Debug)]
struct PartialFrame {
    frag_total: u8,
    /// Codec tag from the first fragment (data or parity). A
    /// mid-frame codec mismatch drops the frame like a keyframe-flag
    /// mismatch would.
    codec: Codec,
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
    /// MEK generation from the first fragment (data or parity); a
    /// mid-frame mismatch drops the frame like a codec mismatch.
    mek_generation: u64,
}

impl PartialFrame {
    fn from_data(
        frag_total: u8,
        keyframe: bool,
        codec: Codec,
        timestamp: u32,
        received_at_ms: u32,
        mek_generation: u64,
    ) -> Self {
        Self {
            frag_total,
            codec,
            keyframe: Some(keyframe),
            timestamp,
            received_at_ms,
            chunks: vec![None; usize::from(frag_total)],
            received_data_count: 0,
            parity_chunks: Vec::new(),
            received_parity_count: 0,
            frame_len: 0,
            mek_generation,
        }
    }

    fn from_parity(
        data_count: u8,
        codec: Codec,
        timestamp: u32,
        received_at_ms: u32,
        mek_generation: u64,
    ) -> Self {
        Self {
            frag_total: data_count,
            codec,
            keyframe: None,
            timestamp,
            received_at_ms,
            chunks: vec![None; usize::from(data_count)],
            received_data_count: 0,
            parity_chunks: Vec::new(),
            received_parity_count: 0,
            frame_len: 0,
            mek_generation,
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

mod complete;
mod ingest;

#[cfg(test)]
mod tests;
