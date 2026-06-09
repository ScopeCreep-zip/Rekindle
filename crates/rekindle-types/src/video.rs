//! Video vocabulary — codec + scalability mode enums shared across the
//! wire envelope (`rekindle-protocol`) and the policy negotiator
//! (`rekindle-video`).
//!
//! Concrete, closed enums by design. No `Other(String)` escape hatch —
//! the codec list is a hard policy decision per release. Adding a new
//! codec or scalability mode is a deliberate schema break, not an
//! opportunistic fallback (memory rule: `feedback_vulnerable_users_no_creative_paths`).

use serde::{Deserialize, Serialize};

/// Video codec identifier. Single concrete variant today; future
/// codecs add variants — never `Other(String)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// VP9 (RFC 6386). Wire string `"vp9"`; full codec parameter
    /// `vp09.00.30.08` is constructed at the encoder layer.
    Vp9,
}

impl Codec {
    /// Stable wire string identifying this codec on the JSON-over-Tauri
    /// surface. Mirrors the historic `Vec<String>` field that this enum
    /// replaces. Add a new arm here when adding a new variant.
    pub fn wire_str(&self) -> &'static str {
        match self {
            Self::Vp9 => "vp9",
        }
    }

    /// Inverse of [`Self::wire_str`]. Returns `None` on unrecognized
    /// input — callers must treat that as a hard error (no fallback).
    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "vp9" => Some(Self::Vp9),
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
        assert_eq!(Codec::Vp9.wire_str(), "vp9");
        assert_eq!(Codec::from_wire_str("vp9"), Some(Codec::Vp9));
        assert_eq!(Codec::from_wire_str("av1"), None);
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
