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
}
