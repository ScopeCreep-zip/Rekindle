//! Video vocabulary — codec + scalability mode enums shared across the
//! wire envelope (`rekindle-protocol`) and the policy negotiator
//! (`rekindle-video`).
//!
//! Concrete, closed enums by design. No `Other(String)` escape hatch —
//! the codec list is a hard policy decision per release. Adding a new
//! codec or scalability mode is a deliberate schema break, not an
//! opportunistic fallback (memory rule: `feedback_vulnerable_users_no_creative_paths`).

use serde::{Deserialize, Serialize};

/// Video codec identifier. Closed set, multi-codec by design (the
/// platform reality: Apple WebKit guarantees H.264 hardware encode but
/// not VP9; WebKitGTK guarantees VP8/VP9 via libvpx but H.264 only via
/// optional GStreamer plugins). Mirrors the RFC 7742 model — a small
/// negotiated set, never `Other(String)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// VP9. Wire string `"vp9"`; WebCodecs codec parameter
    /// `vp09.00.30.08` is constructed at the encoder layer.
    Vp9,
    /// VP8 (RFC 6386) — the 2026 industry interop floor (RFC 7742 MTI;
    /// Signal group calls still run it). WebCodecs string is `"vp8"`.
    Vp8,
    /// H.264 constrained baseline in Annex-B form (`avc1.42E01F` +
    /// `avc: { format: "annexb" }` on the encoder) — SPS/PPS ride the
    /// bitstream, so decoders configure codec-string-only with no
    /// avcC `description`. The Apple-WebKit-guaranteed encoder.
    H264,
}

impl Codec {
    /// Stable wire string identifying this codec on the JSON-over-Tauri
    /// surface. Mirrors the historic `Vec<String>` field that this enum
    /// replaces. Add a new arm here when adding a new variant.
    pub fn wire_str(&self) -> &'static str {
        match self {
            Self::Vp9 => "vp9",
            Self::Vp8 => "vp8",
            Self::H264 => "h264",
        }
    }

    /// Inverse of [`Self::wire_str`]. Returns `None` on unrecognized
    /// input — callers must treat that as a hard error (no fallback).
    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "vp9" => Some(Self::Vp9),
            "vp8" => Some(Self::Vp8),
            "h264" => Some(Self::H264),
            _ => None,
        }
    }

    /// One-byte wire form for the per-fragment codec tag's SIGNING
    /// BYTES (codec-confusion defense). Values match the capnp enum
    /// ordinals (`vp9 @0; vp8 @1; h264 @2;`) — keep them in lock-step.
    pub fn wire_byte(&self) -> u8 {
        match self {
            Self::Vp9 => 0,
            Self::Vp8 => 1,
            Self::H264 => 2,
        }
    }

    /// Inverse of [`Self::wire_byte`].
    pub fn from_wire_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Vp9),
            1 => Some(Self::Vp8),
            2 => Some(Self::H264),
            _ => None,
        }
    }
}

/// VP9 SVC scalability mode. `Flat` means single-layer (no SVC); `L1T2`
/// is the two-temporal-layer encoding used for forward error
/// correction across one dropped frame without re-keyframing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScalabilityMode {
    /// Single-layer; matches WebCodecs `scalabilityMode: undefined`.
    Flat,
    /// 1 spatial × 2 temporal layers; matches WebCodecs `scalabilityMode: "L1T2"`.
    L1T2,
}

impl ScalabilityMode {
    /// Stable wire string. `Flat` round-trips as `"flat"` on the JSON
    /// surface; the WebCodecs API consumes `undefined` for that case —
    /// the frontend handler maps the string back when configuring the
    /// encoder.
    pub fn wire_str(&self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::L1T2 => "l1t2",
        }
    }
}

/// Media capabilities a peer advertises in `MediaCapabilities` when
/// they join a video-bearing channel. Used by the sender to pick a
/// resolution + framerate compatible with the slowest receiver.
///
/// Encode and decode codec sets are SEPARATE lists because WebView
/// engines are asymmetric (Apple WebKit guarantees H.264 hardware
/// encode but not VP9 encode; it can still decode VP9 in many builds).
/// The negotiator picks the LOCAL encoder codec as the first of
/// `encode_codecs` every remote peer can decode — mirroring how WebRTC
/// `getCapabilities` is queried per-direction.
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
    /// Codecs this peer can ENCODE, ordered by preference. May be
    /// empty: a decode-only peer can watch but not send video.
    pub encode_codecs: Vec<Codec>,
    /// Codecs this peer can DECODE, ordered by preference.
    pub decode_codecs: Vec<Codec>,
    /// Whether this peer's `VideoDecoder.configure({ optimizeForLatency: true })`
    /// probe succeeded. The negotiator only enables low-latency mode if
    /// every peer reports `true` (no asymmetric configuration).
    pub supports_optimize_for_latency: bool,
    /// Scalability modes this peer's encoder supports. The negotiator
    /// picks the strongest mode every peer can advertise.
    pub supported_scalability_modes: Vec<ScalabilityMode>,
}

impl MediaCapabilities {
    /// Conservative default for the LOCAL peer before the WebView probe
    /// reports: 480p (854×480) @ 15 fps, VP9-only both directions, flat
    /// scalability, no `optimizeForLatency`. This shape is BROADCAST as
    /// our capabilities — it must never overstate what the local engine
    /// can actually decode.
    pub fn interim_default() -> Self {
        Self {
            max_pixel_count: 854 * 480,
            max_fps: 15,
            encode_codecs: vec![Codec::Vp9],
            decode_codecs: vec![Codec::Vp9],
            supports_optimize_for_latency: false,
            supported_scalability_modes: vec![ScalabilityMode::Flat],
        }
    }

    /// Optimistic placeholder for a REMOTE peer whose `MediaCapabilities`
    /// advertisement hasn't arrived yet. Lists every codec we ship so a
    /// pre-caps joiner never blocks the local encoder pick; it
    /// self-heals when the real advertisement lands (the aggregator
    /// recomputes on `on_peer_caps_received`) and receivers create
    /// decoders from per-fragment tags regardless. Never broadcast as
    /// LOCAL caps — see `interim_default` for that.
    pub fn optimistic_peer_default() -> Self {
        Self {
            max_pixel_count: 854 * 480,
            max_fps: 15,
            encode_codecs: vec![Codec::Vp9, Codec::Vp8, Codec::H264],
            decode_codecs: vec![Codec::Vp9, Codec::Vp8, Codec::H264],
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

/// Decoder tuning the negotiator hands to the local decoder pipeline.
/// Carries NO codec: decoders are created from the per-fragment codec
/// tag (the RTP payload-type analog), never from session config.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecoderConstraints {
    pub optimize_for_latency: bool,
}

/// The PER-NODE encoder + decoder configuration produced by
/// `rekindle_video::policy::negotiate_session_config`. Asymmetric by
/// design: each node picks its own encoder codec (first of its
/// `encode_codecs` every remote can decode), so a Mac may send H.264
/// while a Pop!_OS peer in the same call sends VP9. Resolution/fps/latency
/// are still min-merged room-wide.
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
    fn codec_wire_roundtrip() {
        for codec in [Codec::Vp9, Codec::Vp8, Codec::H264] {
            assert_eq!(Codec::from_wire_str(codec.wire_str()), Some(codec));
            assert_eq!(Codec::from_wire_byte(codec.wire_byte()), Some(codec));
        }
        assert_eq!(Codec::Vp9.wire_str(), "vp9");
        assert_eq!(Codec::Vp8.wire_str(), "vp8");
        assert_eq!(Codec::H264.wire_str(), "h264");
        assert_eq!(Codec::from_wire_str("av1"), None);
        assert_eq!(Codec::from_wire_byte(99), None);
        // wire_byte values are the capnp enum ordinals — lock-step.
        assert_eq!(Codec::Vp9.wire_byte(), 0);
        assert_eq!(Codec::Vp8.wire_byte(), 1);
        assert_eq!(Codec::H264.wire_byte(), 2);
    }

    #[test]
    fn new_codec_serde_lowercase() {
        assert_eq!(serde_json::to_string(&Codec::Vp8).unwrap(), "\"vp8\"");
        assert_eq!(serde_json::to_string(&Codec::H264).unwrap(), "\"h264\"");
        assert_eq!(
            serde_json::from_str::<Codec>("\"h264\"").unwrap(),
            Codec::H264
        );
    }

    #[test]
    fn scalability_mode_wire_strings() {
        assert_eq!(ScalabilityMode::Flat.wire_str(), "flat");
        assert_eq!(ScalabilityMode::L1T2.wire_str(), "l1t2");
    }

    #[test]
    fn codec_serde_lowercase() {
        let json = serde_json::to_string(&Codec::Vp9).unwrap();
        assert_eq!(json, "\"vp9\"");
        let back: Codec = serde_json::from_str("\"vp9\"").unwrap();
        assert_eq!(back, Codec::Vp9);
    }

    #[test]
    fn scalability_serde_lowercase() {
        let json = serde_json::to_string(&ScalabilityMode::L1T2).unwrap();
        assert_eq!(json, "\"l1t2\"");
        let back: ScalabilityMode = serde_json::from_str("\"flat\"").unwrap();
        assert_eq!(back, ScalabilityMode::Flat);
    }

    #[test]
    fn capabilities_interim_default_under_budget() {
        let caps = MediaCapabilities::interim_default();
        assert!(caps.max_pixel_count <= 480 * 1280); // 720p ceiling
        assert_eq!(caps.max_fps, 15);
        assert_eq!(caps.encode_codecs, vec![Codec::Vp9]);
        assert_eq!(caps.decode_codecs, vec![Codec::Vp9]);
        assert!(!caps.supports_optimize_for_latency);
        assert_eq!(
            caps.supported_scalability_modes,
            vec![ScalabilityMode::Flat]
        );
    }

    #[test]
    fn optimistic_peer_default_lists_all_shipped_codecs() {
        let caps = MediaCapabilities::optimistic_peer_default();
        assert_eq!(
            caps.decode_codecs,
            vec![Codec::Vp9, Codec::Vp8, Codec::H264],
            "remote placeholder must never block the local encoder pick"
        );
        assert_eq!(caps.encode_codecs, caps.decode_codecs);
    }

    /// Pins the `camelCase` wire contract at its new Tier 1 home. These
    /// structs cross the wire (gossip control / events / Cap'n Proto
    /// mapping), so the JSON key names are load-bearing — a rename here
    /// silently breaks the frontend TS contract and remote peers.
    #[test]
    fn media_capabilities_camel_case_wire() {
        let json = serde_json::to_value(MediaCapabilities::interim_default()).unwrap();
        let obj = json.as_object().unwrap();
        let keys: std::collections::BTreeSet<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "maxPixelCount",
                "maxFps",
                "encodeCodecs",
                "decodeCodecs",
                "supportsOptimizeForLatency",
                "supportedScalabilityModes",
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<&str>>()
        );
        // Codec enum values ride through as lowercase strings.
        assert_eq!(obj["encodeCodecs"], serde_json::json!(["vp9"]));
        assert_eq!(
            obj["supportedScalabilityModes"],
            serde_json::json!(["flat"])
        );
    }

    #[test]
    fn bandwidth_estimate_camel_case_wire() {
        let est = BandwidthEstimate {
            kbps: 600,
            window_secs: 2,
            loss_q8: 13,
        };
        assert_eq!(
            serde_json::to_value(&est).unwrap(),
            serde_json::json!({
                "kbps": 600,
                "windowSecs": 2,
                "lossQ8": 13,
            })
        );
    }

    #[test]
    fn session_video_config_camel_case_wire() {
        let cfg = SessionVideoConfig {
            encoder: EncoderConstraints {
                codec: Codec::Vp9,
                max_width: 854,
                max_height: 480,
                max_fps: 15,
                scalability_mode: ScalabilityMode::Flat,
            },
            decoder: DecoderConstraints {
                optimize_for_latency: false,
            },
        };
        assert_eq!(
            serde_json::to_value(&cfg).unwrap(),
            serde_json::json!({
                "encoder": {
                    "codec": "vp9",
                    "maxWidth": 854,
                    "maxHeight": 480,
                    "maxFps": 15,
                    "scalabilityMode": "flat",
                },
                "decoder": { "optimizeForLatency": false },
            })
        );
    }
}
