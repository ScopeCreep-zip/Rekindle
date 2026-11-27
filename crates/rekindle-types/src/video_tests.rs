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
