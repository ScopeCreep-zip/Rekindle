//! Video & screen-share fragmentation and reassembly per architecture
//! §10.6. Pure-logic crate — no codec FFI, no Tauri, no I/O. The actual
//! VP9 encode/decode plugs in via the `VideoCodec` trait at the
//! application layer; this crate handles only the on-the-wire framing
//! (≤28 KB payload chunks, FEC-friendly indexing, per-stream
//! reassembly buffer with bounded memory).

pub mod fragment;
pub mod reassembler;

pub use fragment::{
    fragment_frame, fragment_frame_with_fec, fragment_signing_bytes, parity_signing_bytes,
    reconstruct_frame, FecFragments, FragmentError, VideoFragment, VideoParityFragment,
    FRAGMENT_PAYLOAD_LIMIT, MAX_FRAGMENTS_PER_FRAME, STREAM_ID_LEN,
};
pub use reassembler::{ReassembledFrame, Reassembler, ReassemblerError};

/// Media capabilities a peer advertises in `MediaCapabilities` when
/// they join a video-bearing channel. Used by the sender to pick a
/// resolution + framerate compatible with the slowest receiver.
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
    pub codecs: Vec<String>,
}

impl MediaCapabilities {
    /// Conservative default suitable for the interim §10.6 budget:
    /// 480p (854×480) @ 15 fps, VP9 only.
    pub fn interim_default() -> Self {
        Self {
            max_pixel_count: 854 * 480,
            max_fps: 15,
            codecs: vec!["vp9".into()],
        }
    }
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
        assert_eq!(caps.codecs, vec!["vp9".to_string()]);
    }
}
