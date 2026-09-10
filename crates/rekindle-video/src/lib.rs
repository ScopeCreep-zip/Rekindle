//! Video & screen-share fragmentation and reassembly per architecture
//! §10.6. Pure-logic crate — no codec FFI, no Tauri, no I/O. The actual
//! VP9 encode/decode plugs in via the `VideoCodec` trait at the
//! application layer; this crate handles only the on-the-wire framing
//! (≤28 KB payload chunks, FEC-friendly indexing, per-stream
//! reassembly buffer with bounded memory).
//!
//! The pure video-vocabulary structs (`MediaCapabilities`,
//! `EncoderConstraints`, `DecoderConstraints`, `SessionVideoConfig`,
//! `BandwidthEstimate`) live in Tier 1 `rekindle-types` alongside
//! `Codec`/`ScalabilityMode` and are re-exported here so this crate's
//! public API is unchanged. A Tier-1 event family cannot depend upward
//! on this Tier-7 crate, which is why the vocabulary lives below.

pub mod budget;
pub mod codec;
pub mod deps;
pub mod error;
pub mod fragment;
pub mod pacer;
pub mod policy;
pub mod reassembler;
pub mod reassembly_state;
pub mod receive;
pub mod send;
pub mod send_pacer;
pub mod stream_id;

#[cfg(test)]
mod test_mock;

pub use budget::{
    encoder_target_kbps, target_from_feedback, target_from_feedback_ceiled, wire_feedback_kbps,
    START_PAYLOAD_SHARE_Q10, VIDEO_MAX_KBPS, VIDEO_MAX_KBPS_VOICE_PRESSURE, VIDEO_MIN_KBPS,
    VIDEO_START_KBPS,
};
pub use codec::{EncodedVideoFrame, RawFrame, VideoEncoder};
pub use deps::{VideoDeps, VideoEvent};
pub use error::VideoError;
pub use fragment::{
    fragment_frame, fragment_frame_with_fec, fragment_signing_bytes, parity_signing_bytes,
    reconstruct_frame, FecFragments, FragmentError, VideoFragment, VideoParityFragment,
    FRAGMENT_PAYLOAD_LIMIT, MAX_FRAGMENTS_PER_FRAME, STREAM_ID_LEN,
};
pub use pacer::{PacedFrame, PacerStats, VideoPacer};
pub use policy::negotiate_session_config;
pub use reassembler::{ReassembledFrame, Reassembler, ReassemblerError};
pub use reassembly_state::VideoReassemblyState;
pub use receive::{handle_video_payload, video_payload_channel};
pub use rekindle_types::video::{
    BandwidthEstimate, Codec, DecoderConstraints, EncoderConstraints, MediaCapabilities,
    ScalabilityMode, SessionVideoConfig,
};
pub use send::{build_video_frame, VideoFrameSend};
pub use send_pacer::run_video_pacer;
pub use stream_id::derive_stream_id;
