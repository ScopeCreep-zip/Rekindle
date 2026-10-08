//! Pure-logic video session negotiator.
//!
//! Computes the `SessionVideoConfig` the LOCAL node encodes and decodes
//! against. No I/O, no time, no randomness — but unlike the pre-split
//! negotiator the output is PER-NODE, not room-symmetric: each node
//! picks its own encoder codec (first of its `encode_codecs` that every
//! remote peer can decode), so a Mac sending H.264 and a Pop!_OS peer
//! sending VP9 in the same call is the correct steady state. Receivers
//! demux by the per-fragment codec tag, never by this config.
//!
//! Plan-doc invariants:
//! - encoder codec: first of `local.encode_codecs` contained in EVERY
//!   peer's `decode_codecs`; zero peers → local's first preference;
//!   empty intersection or no local encoder → `None` (caller surfaces
//!   `VideoCodecIncompatible` / decode-only)
//! - min of (max_width, max_height, max_fps) across local + peers
//! - `optimizeForLatency` only when ALL (local + peers) report support
//! - `ScalabilityMode::L1T2` only for VP9 AND when ALL list it (the
//!   WebView probe only validates L1T2 against the VP9 encoder)
//! - frame budget: `max_width * max_height * 3 <= 7 MiB`. Panic in
//!   debug builds (this means the caller passed inconsistent caps);
//!   in release, clamp width and height down to the budget.

use crate::{
    Codec, DecoderConstraints, EncoderConstraints, MediaCapabilities, ScalabilityMode,
    SessionVideoConfig,
};

/// The fragmenter's hard transport ceiling for ONE encoded frame:
/// `FRAGMENT_PAYLOAD_LIMIT × MAX_FRAGMENTS_PER_FRAME` (≈ 1 MiB at the
/// 4 KiB budget). Derived, never a literal — this constant silently
/// lied once when the fragment budget changed under it.
const TRANSPORT_FRAME_CEILING_BYTES: u64 =
    (crate::fragment::FRAGMENT_PAYLOAD_LIMIT * crate::fragment::MAX_FRAGMENTS_PER_FRAME) as u64;

/// Explicit worst-case compression floor for the raw→encoded proxy:
/// VP8/VP9 intra under our realtime CBR + quantizer caps stays well
/// above 3:1 versus 3 B/px raw even on noise (pathological observed:
/// 4.2:1 — a 294 KB keyframe at 854×480). The budget check validates
/// RAW bytes, so the ceiling is scaled up by this floor.
const COMPRESSION_FLOOR: u64 = 3;

/// Total per-frame RAW byte budget (≈ 3 MiB): a config passing
/// `width × height × 3 ≤ ceiling × floor` cannot produce an encoded
/// frame that overruns the fragmenter. 854×480 and 1280×720 pass;
/// 1080p+ clamps.
const FRAME_BUDGET_BYTES: u64 = TRANSPORT_FRAME_CEILING_BYTES * COMPRESSION_FLOOR;
const BYTES_PER_PIXEL: u64 = 3;

/// Compute the local node's `SessionVideoConfig` against a set of
/// remote peer caps.
///
/// `peers` may be empty — the local participant is alone in the
/// channel and the config follows their own caps.
///
/// Returns `None` when no encoder codec works: either the local node
/// has no encoder at all (`encode_codecs` is empty — a decode-only
/// platform, still a valid watcher) or no local encode codec is
/// decodable by every peer (genuine incompatibility — the caller emits
/// `VideoCodecIncompatible`).
pub fn negotiate_session_config(
    local: &MediaCapabilities,
    peers: &[MediaCapabilities],
) -> Option<SessionVideoConfig> {
    let codec = pick_encoder_codec(local, peers)?;

    // Minimum resolution / fps across local + peers. width × height is
    // approximated from `max_pixel_count` assuming the 16:9 aspect ratio
    // that the WebView capture pipeline produces. The reverse map
    // (pixel_count → width × height) sticks to the standard ladder so
    // the encoder gets clean numbers (e.g. 1280×720, 854×480).
    let max_pixel_count = all_caps(local, peers)
        .map(|c| c.max_pixel_count)
        .min()
        .expect("iterator includes local — never empty");
    let (mut max_width, mut max_height) = resolution_from_pixel_count(max_pixel_count);
    let max_fps = u32::from(
        all_caps(local, peers)
            .map(|c| c.max_fps)
            .min()
            .expect("iterator includes local — never empty"),
    );

    // Frame budget assertion + saturating clamp.
    let mut frame_bytes = u64::from(max_width) * u64::from(max_height) * BYTES_PER_PIXEL;
    if frame_bytes > FRAME_BUDGET_BYTES {
        debug_assert!(
            false,
            "negotiate_session_config: frame budget exceeded: {max_width}x{max_height} × {BYTES_PER_PIXEL} = {frame_bytes} > {FRAME_BUDGET_BYTES}"
        );
        // Release fallback: clamp width and height down (halve each
        // dimension until inside the budget). Cannot loop forever
        // because `max_width / 2^n` reaches 0 after `log2(max_width)`
        // iterations.
        while frame_bytes > FRAME_BUDGET_BYTES && max_width > 0 && max_height > 0 {
            max_width /= 2;
            max_height /= 2;
            frame_bytes = u64::from(max_width) * u64::from(max_height) * BYTES_PER_PIXEL;
        }
    }

    let scalability_mode = pick_scalability_mode(codec, local, peers);
    let optimize_for_latency = all_caps(local, peers).all(|c| c.supports_optimize_for_latency);

    Some(SessionVideoConfig {
        encoder: EncoderConstraints {
            codec,
            max_width,
            max_height,
            max_fps,
            scalability_mode,
        },
        decoder: DecoderConstraints {
            optimize_for_latency,
        },
    })
}

/// Local + every remote peer, as one iterator. The min-merge invariants
/// (resolution, fps, latency, L1T2) all range over this set.
fn all_caps<'a>(
    local: &'a MediaCapabilities,
    peers: &'a [MediaCapabilities],
) -> impl Iterator<Item = &'a MediaCapabilities> {
    std::iter::once(local).chain(peers.iter())
}

/// First codec from the local ENCODE preference list that every peer
/// can DECODE. With zero peers the intersection is vacuous and the
/// local first preference wins. `None` = no local encoder, or no
/// mutually decodable codec (the caller decides which it is by
/// checking `local.encode_codecs.is_empty()`).
fn pick_encoder_codec(local: &MediaCapabilities, peers: &[MediaCapabilities]) -> Option<Codec> {
    local
        .encode_codecs
        .iter()
        .copied()
        .find(|candidate| peers.iter().all(|p| p.decode_codecs.contains(candidate)))
}

/// `ScalabilityMode::L1T2` only when the picked codec is VP9 and every
/// participant (local + peers) lists it; otherwise `Flat`. The WebView
/// probe validates L1T2 against the VP9 encoder only — forcing `Flat`
/// for VP8/H.264 keeps `encoder.configure()` valid on WebKit.
fn pick_scalability_mode(
    codec: Codec,
    local: &MediaCapabilities,
    peers: &[MediaCapabilities],
) -> ScalabilityMode {
    let all_l1t2 = codec == Codec::Vp9
        && all_caps(local, peers).all(|c| {
            c.supported_scalability_modes
                .contains(&ScalabilityMode::L1T2)
        });
    if all_l1t2 {
        ScalabilityMode::L1T2
    } else {
        ScalabilityMode::Flat
    }
}

/// Reverse-map `max_pixel_count` → `(width, height)` for the canonical
/// 16:9 resolutions the WebView capture pipeline produces. Picks the
/// largest standard resolution whose pixel count fits the cap; falls
/// back to a 16:9 derivation if no standard fits (e.g. very small caps
/// in tests).
fn resolution_from_pixel_count(max_pixel_count: u32) -> (u32, u32) {
    // Architecture §10.6 ladder, largest first.
    const LADDER: &[(u32, u32)] = &[
        (1920, 1080),
        (1280, 720),
        (854, 480),
        (640, 360),
        (426, 240),
    ];
    for (w, h) in LADDER {
        if w * h <= max_pixel_count {
            return (*w, *h);
        }
    }
    // Below the smallest ladder rung — derive a 16:9-ish resolution
    // from the cap using only integer math. `isqrt` (stable since
    // Rust 1.84) gives an exact floor(sqrt(_)) so the result never
    // overshoots the budget.
    let pixels = u64::from(max_pixel_count);
    // 16:9 → width² ≈ pixels × 16 / 9; saturating to keep u64 closed.
    let width_sq = pixels.saturating_mul(16) / 9;
    let width = width_sq.isqrt().max(1);
    let height = (pixels / width).max(1);
    // Both `width` and `height` are bounded above by `pixels` (≤ u32::MAX),
    // so `u32::try_from` only ever fails when the input cap itself
    // exceeds u32::MAX — which it can't, because `max_pixel_count` is u32.
    let width_u32 = u32::try_from(width).unwrap_or(u32::MAX);
    let height_u32 = u32::try_from(height).unwrap_or(u32::MAX);
    (width_u32, height_u32)
}

// ── unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
}
