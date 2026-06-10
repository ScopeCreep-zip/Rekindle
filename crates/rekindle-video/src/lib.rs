//! Video & screen-share fragmentation and reassembly per architecture
//! §10.6. Pure-logic crate — no codec FFI, no Tauri, no I/O. The actual
//! VP9 encode/decode plugs in via the `VideoCodec` trait at the
//! application layer; this crate handles only the on-the-wire framing
//! (≤28 KB payload chunks, FEC-friendly indexing, per-stream
//! reassembly buffer with bounded memory).

pub mod deps;
pub mod error;
pub mod fragment;
pub mod policy;
pub mod reassembler;
pub mod reassembly_state;
pub mod receive;
pub mod send;
pub mod stream_id;

#[cfg(test)]
mod test_mock;

pub use deps::{VideoDeps, VideoEvent};
pub use error::VideoError;
pub use fragment::{
    fragment_frame, fragment_frame_with_fec, fragment_signing_bytes, parity_signing_bytes,
    reconstruct_frame, FecFragments, FragmentError, VideoFragment, VideoParityFragment,
    FRAGMENT_PAYLOAD_LIMIT, MAX_FRAGMENTS_PER_FRAME, STREAM_ID_LEN,
};
pub use policy::negotiate_session_config;
pub use reassembler::{ReassembledFrame, Reassembler, ReassemblerError};
pub use reassembly_state::VideoReassemblyState;
pub use receive::{handle_video_payload, video_payload_channel};
pub use rekindle_types::video::{Codec, ScalabilityMode};
pub use send::{send_video_frame, VideoFrameSend};
pub use stream_id::derive_stream_id;

/// Media capabilities a peer advertises in `MediaCapabilities` when
/// they join a video-bearing channel. Used by the sender to pick a
/// resolution + framerate compatible with the slowest receiver.
///
/// Carries typed `Codec` and `ScalabilityMode` lists (no `Vec<String>`
/// fallback) plus the WebCodecs `optimizeForLatency` flag — the three
/// inputs `negotiate_session_config` needs to pick one room-wide
/// encoder/decoder config.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCapabilities {
    /// Highest resolution (in pixels) the peer can decode without
    /// dropping frames. Architecture §10.6 caps the sender at 480p
    /// (854×480 = 410k pixels) until the veilid-media RFC lands.
    pub max_pixel_count: u32,
    /// Highest framerate the peer can decode. Capped at 15fps in
    /// the interim phase.
    pub max_fps: u8,
    /// Codecs this peer can decode, ordered by preference.
    pub codecs: Vec<Codec>,
    /// Whether this peer's `VideoDecoder.configure({ optimizeForLatency: true })`
    /// probe succeeded. The negotiator only enables low-latency mode if
    /// every peer reports `true` (no asymmetric configuration).
    pub supports_optimize_for_latency: bool,
    /// Scalability modes this peer's encoder supports. The negotiator
    /// picks the strongest mode every peer can advertise.
    pub supported_scalability_modes: Vec<ScalabilityMode>,
}

impl MediaCapabilities {
    /// Conservative default suitable for the interim §10.6 budget:
    /// 480p (854×480) @ 15 fps, VP9 only, flat scalability, no
    /// `optimizeForLatency` until the WebView probe confirms it.
    pub fn interim_default() -> Self {
        Self {
            max_pixel_count: 854 * 480,
            max_fps: 15,
            codecs: vec![Codec::Vp9],
            supports_optimize_for_latency: false,
            supported_scalability_modes: vec![ScalabilityMode::Flat],
        }
    }
}

/// Encoder configuration the negotiator hands to every peer's encoder.
/// Resolution is expressed in width/height (instead of `max_pixel_count`)
/// so the encoder can configure without a square-root.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderConstraints {
    pub codec: Codec,
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
    pub scalability_mode: ScalabilityMode,
}

/// Decoder configuration the negotiator hands to every peer's decoder.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecoderConstraints {
    pub codec: Codec,
    pub optimize_for_latency: bool,
}

/// The room-wide encoder + decoder configuration produced by
/// [`policy::negotiate_session_config`]. Symmetric: every peer encodes
/// and decodes against the same shape.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionVideoConfig {
    pub encoder: EncoderConstraints,
    pub decoder: DecoderConstraints,
}

/// Bandwidth feedback from a receiver. Architecture §10.6 specifies
/// 500ms cadence; senders adjust their VP9 bitrate to fit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BandwidthEstimate {
    /// Receiver's measured downstream bandwidth in kbps.
    pub kbps: u32,
    /// Window duration (seconds) the kbps was averaged over.
    pub window_secs: u8,
    /// Fraction of fragments lost in the same window (0–255 → 0..=1).
    pub loss_q8: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_interim_default_under_budget() {
        let caps = MediaCapabilities::interim_default();
        assert!(caps.max_pixel_count <= 480 * 1280); // 720p ceiling
        assert_eq!(caps.max_fps, 15);
        assert_eq!(caps.codecs, vec![Codec::Vp9]);
        assert!(!caps.supports_optimize_for_latency);
        assert_eq!(
            caps.supported_scalability_modes,
            vec![ScalabilityMode::Flat]
        );
    }
}
