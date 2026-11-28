use super::*;

fn mac_caps() -> MediaCapabilities {
    // Mac WKWebView style: 720p @ 30, H.264 hardware encode only,
    // decodes H.264 + VP9, L1T2 supported, optimizeForLatency works.
    MediaCapabilities {
        max_pixel_count: 1280 * 720,
        max_fps: 30,
        encode_codecs: vec![Codec::H264],
        decode_codecs: vec![Codec::Vp9, Codec::H264],
        supports_optimize_for_latency: true,
        supported_scalability_modes: vec![ScalabilityMode::Flat, ScalabilityMode::L1T2],
    }
}

fn pop_caps() -> MediaCapabilities {
    // Pop!_OS WebKitGTK style: 1080p @ 30, VP9/VP8 via libvpx +
    // H.264 decode via GStreamer, no L1T2, optimizeForLatency probe
    // fails.
    MediaCapabilities {
        max_pixel_count: 1920 * 1080,
        max_fps: 30,
        encode_codecs: vec![Codec::Vp9, Codec::Vp8],
        decode_codecs: vec![Codec::Vp9, Codec::Vp8, Codec::H264],
        supports_optimize_for_latency: false,
        supported_scalability_modes: vec![ScalabilityMode::Flat],
    }
}

#[test]
fn zero_peers_uses_local_first_preference() {
    let cfg = negotiate_session_config(&MediaCapabilities::interim_default(), &[])
        .expect("local-only negotiation always succeeds with an encoder");
    // Interim default ladder rung: 854×480.
    assert_eq!(cfg.encoder.codec, Codec::Vp9);
    assert_eq!(cfg.encoder.max_width, 854);
    assert_eq!(cfg.encoder.max_height, 480);
    assert_eq!(cfg.encoder.max_fps, 15);
    assert_eq!(cfg.encoder.scalability_mode, ScalabilityMode::Flat);
    assert!(!cfg.decoder.optimize_for_latency);
}

#[test]
fn single_local_uses_own_caps() {
    let cfg = negotiate_session_config(&mac_caps(), &[]).expect("alone in channel");
    // Largest ladder rung ≤ 1280×720 = 1280×720 itself.
    assert_eq!(cfg.encoder.codec, Codec::H264, "local first preference");
    assert_eq!(cfg.encoder.max_width, 1280);
    assert_eq!(cfg.encoder.max_height, 720);
    assert_eq!(cfg.encoder.max_fps, 30);
    // H.264 picked → L1T2 suppressed even though the local engine
    // lists it (L1T2 is only probed against VP9).
    assert_eq!(cfg.encoder.scalability_mode, ScalabilityMode::Flat);
    assert!(cfg.decoder.optimize_for_latency);
}

#[test]
fn asymmetric_pick_is_per_node() {
    // Mac (encodes h264 only) facing Pop (decodes vp9/vp8/h264):
    // Mac's pick is H264. Pop facing Mac (decodes vp9+h264): Pop's
    // pick is its first preference VP9. Each node encodes its own
    // codec — the correct steady state, not an error.
    let mac_cfg = negotiate_session_config(&mac_caps(), &[pop_caps()]).expect("compatible");
    assert_eq!(mac_cfg.encoder.codec, Codec::H264);

    let pop_cfg = negotiate_session_config(&pop_caps(), &[mac_caps()]).expect("compatible");
    assert_eq!(pop_cfg.encoder.codec, Codec::Vp9);
}

#[test]
fn mixed_caps_min_merge_resolution_latency() {
    let cfg = negotiate_session_config(&mac_caps(), &[pop_caps()]).expect("compatible");
    assert_eq!(cfg.encoder.max_width, 1280);
    assert_eq!(cfg.encoder.max_height, 720);
    assert_eq!(cfg.encoder.max_fps, 30);
    assert!(
        !cfg.decoder.optimize_for_latency,
        "optimizeForLatency must drop out when ANY participant reports unsupported"
    );
}

#[test]
fn preference_order_respected() {
    // Local prefers vp9 then h264; peer decodes only h264 → h264.
    let local = MediaCapabilities {
        encode_codecs: vec![Codec::Vp9, Codec::H264],
        ..MediaCapabilities::interim_default()
    };
    let peer = MediaCapabilities {
        decode_codecs: vec![Codec::H264],
        ..MediaCapabilities::interim_default()
    };
    let cfg = negotiate_session_config(&local, &[peer]).expect("h264 bridges");
    assert_eq!(cfg.encoder.codec, Codec::H264);
}

#[test]
fn empty_intersection_returns_none() {
    let local = MediaCapabilities {
        encode_codecs: vec![Codec::Vp9],
        ..MediaCapabilities::interim_default()
    };
    let peer = MediaCapabilities {
        decode_codecs: vec![Codec::H264],
        ..MediaCapabilities::interim_default()
    };
    assert!(negotiate_session_config(&local, &[peer]).is_none());
}

#[test]
fn decode_only_local_returns_none() {
    let local = MediaCapabilities {
        encode_codecs: vec![],
        ..MediaCapabilities::interim_default()
    };
    assert!(negotiate_session_config(&local, &[]).is_none());
    assert!(negotiate_session_config(&local, &[pop_caps()]).is_none());
}

#[test]
fn l1t2_requires_vp9_and_unanimous_support() {
    // Two Mac-style peers but with VP9 encode: all list L1T2 → kept.
    let vp9_mac = MediaCapabilities {
        encode_codecs: vec![Codec::Vp9],
        decode_codecs: vec![Codec::Vp9],
        ..mac_caps()
    };
    let cfg =
        negotiate_session_config(&vp9_mac, std::slice::from_ref(&vp9_mac)).expect("compatible");
    assert_eq!(cfg.encoder.codec, Codec::Vp9);
    assert_eq!(cfg.encoder.scalability_mode, ScalabilityMode::L1T2);

    // Same pair but one participant lacks L1T2 → Flat.
    let cfg = negotiate_session_config(&vp9_mac, &[pop_caps()]).expect("compatible");
    assert_eq!(
        cfg.encoder.scalability_mode,
        ScalabilityMode::Flat,
        "L1T2 must drop out when ANY participant doesn't list it"
    );
}

#[test]
fn pixel_count_floor_clamps_to_smallest_ladder_rung() {
    // Tiny cap below the smallest ladder rung → derived 16:9
    // resolution, not 426×240. Sanity test the fallback path.
    let tiny = MediaCapabilities {
        max_pixel_count: 100,
        max_fps: 10,
        ..MediaCapabilities::interim_default()
    };
    let cfg = negotiate_session_config(&tiny, &[]).expect("alone in channel");
    // Derived width × height should be ≤ 100.
    assert!(cfg.encoder.max_width * cfg.encoder.max_height <= 100);
    assert_eq!(cfg.encoder.max_fps, 10);
}
