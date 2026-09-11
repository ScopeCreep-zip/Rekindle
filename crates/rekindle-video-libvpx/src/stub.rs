//! Feature-OFF path (`not(feature = "libvpx")`).
//!
//! Pure-Rust stand-ins with the **same public API** as the real
//! [`imp::LibvpxEncoder`](crate::imp) and
//! [`imp_decoder::LibvpxDecoder`](crate::imp_decoder), carrying no C
//! dependency. Every fallible entry point returns
//! [`VideoError::Unsupported`] naming the fix ("rebuild with
//! `--features libvpx`"); the infallible levers (`force_keyframe`,
//! `set_bitrate`) are no-ops. This keeps callers that reference
//! `LibvpxEncoder` / `LibvpxDecoder` compiling on a machine without libvpx
//! and lets them degrade gracefully at runtime instead of failing to
//! build.

use rekindle_types::video::Codec;
use rekindle_video::codec::{EncodedVideoFrame, RawFrame, VideoDecoder, VideoEncoder};
use rekindle_video::error::VideoError;
use rekindle_video::EncoderConstraints;

const NOT_BUILT: &str = "rekindle-video-libvpx was built without the `libvpx` feature; \
     rebuild with `--features libvpx` (system libvpx + libclang, VPX_STATIC=1 to static-link)";

/// Stub libvpx encoder — present so downstream code type-checks without
/// the `libvpx` feature. It is never successfully constructed:
/// [`Self::new`] always returns [`VideoError::Unsupported`].
pub struct LibvpxEncoder {
    codec: Codec,
}

impl LibvpxEncoder {
    /// Always fails without the `libvpx` feature. `codec` is the initial
    /// codec the real encoder would emit; retained only so the shared API
    /// signature matches [`imp::LibvpxEncoder::new`](crate::imp).
    pub fn new(codec: Codec) -> Result<Self, VideoError> {
        // Construct the value (keeps the field + constructor reachable for
        // the dead-code lint) and log it — the stub never yields a live
        // encoder, so this instance is dropped and `new` reports the fix.
        let encoder = Self { codec };
        tracing::debug!(
            target: "rekindle_video_libvpx",
            codec = encoder.codec.wire_str(),
            "libvpx feature not enabled — LibvpxEncoder is a stub"
        );
        Err(VideoError::Unsupported(NOT_BUILT.to_string()))
    }
}

impl VideoEncoder for LibvpxEncoder {
    fn configure(&mut self, _cfg: &EncoderConstraints) -> Result<(), VideoError> {
        Err(VideoError::Unsupported(NOT_BUILT.to_string()))
    }

    fn encode(&mut self, _frame: RawFrame) -> Result<Option<EncodedVideoFrame>, VideoError> {
        Err(VideoError::Unsupported(NOT_BUILT.to_string()))
    }

    fn force_keyframe(&mut self) {}

    fn set_bitrate(&mut self, _kbps: u32) {}

    fn codec(&self) -> Codec {
        self.codec
    }
}

/// Stub libvpx decoder — present so downstream code type-checks without
/// the `libvpx` feature. It is never successfully constructed:
/// [`Self::new`] always returns [`VideoError::Unsupported`].
pub struct LibvpxDecoder {
    codec: Codec,
}

impl LibvpxDecoder {
    /// Always fails without the `libvpx` feature. `codec` is the codec the
    /// real decoder would consume; retained only so the shared API
    /// signature matches [`imp_decoder::LibvpxDecoder::new`](crate::imp_decoder).
    pub fn new(codec: Codec) -> Result<Self, VideoError> {
        // Construct the value (keeps the field + constructor reachable for
        // the dead-code lint) and log it — the stub never yields a live
        // decoder, so this instance is dropped and `new` reports the fix.
        let decoder = Self { codec };
        tracing::debug!(
            target: "rekindle_video_libvpx",
            codec = decoder.codec.wire_str(),
            "libvpx feature not enabled — LibvpxDecoder is a stub"
        );
        Err(VideoError::Unsupported(NOT_BUILT.to_string()))
    }
}

impl VideoDecoder for LibvpxDecoder {
    fn decode(&mut self, _frame: &EncodedVideoFrame) -> Result<Option<RawFrame>, VideoError> {
        Err(VideoError::Unsupported(NOT_BUILT.to_string()))
    }

    fn codec(&self) -> Codec {
        self.codec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_reports_unsupported_without_feature() {
        match LibvpxEncoder::new(Codec::Vp9) {
            Err(VideoError::Unsupported(msg)) => assert!(msg.contains("libvpx")),
            Err(other) => panic!("expected Unsupported, got a different error: {other:?}"),
            Ok(_) => panic!("stub new() must never succeed without the libvpx feature"),
        }
    }

    #[test]
    fn decoder_new_reports_unsupported_without_feature() {
        match LibvpxDecoder::new(Codec::Vp9) {
            Err(VideoError::Unsupported(msg)) => assert!(msg.contains("libvpx")),
            Err(other) => panic!("expected Unsupported, got a different error: {other:?}"),
            Ok(_) => panic!("stub new() must never succeed without the libvpx feature"),
        }
    }
}
