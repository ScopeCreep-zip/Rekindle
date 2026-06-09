//! Pure-logic video session negotiator.
//!
//! Computes the single `SessionVideoConfig` that every peer in a video
//! channel must encode and decode against. No I/O, no time, no
//! randomness — given the same inputs, every peer derives the same
//! output, so the backend on each device picks identical encoder and
//! decoder settings without any coordinator.
//!
//! Plan-doc invariants:
//! - intersection of supported codecs; pick the first codec all peers list
//! - min of (max_width, max_height, max_fps) across peers
//! - `optimizeForLatency` only when ALL peers report support
//! - `ScalabilityMode::L1T2` only when ALL peers list it
//! - frame budget: `max_width * max_height * 3 <= 7 MiB`. Panic in
//!   debug builds (this means the caller passed inconsistent caps);
//!   in release, clamp width and height down to the budget.

use crate::{
    Codec, DecoderConstraints, EncoderConstraints, MediaCapabilities, ScalabilityMode,
    SessionVideoConfig,
};

/// Total per-frame byte budget. `FRAGMENT_PAYLOAD_LIMIT (28 KiB) ×
/// MAX_FRAGMENTS_PER_FRAME (256)` = 7 MiB ceiling, asserted by the
/// fragmenter. A frame at 3 bytes/pixel (RGB / planar 4:4:4) cannot
/// exceed this without overrunning the transport.
const FRAME_BUDGET_BYTES: u64 = 7 * 1024 * 1024;
const BYTES_PER_PIXEL: u64 = 3;

/// Compute the room-wide `SessionVideoConfig` for a set of peer caps.
///
/// `peers` may be empty — in that case the local participant is alone
/// in the channel and the negotiator returns the interim default
/// (a backend invariant we can rely on because the caller always seeds
/// at least the local peer's caps via [`MediaCapabilities::interim_default`]
/// before calling).
///
/// # Panics
///
/// - In debug builds, if any peer reports an empty codec list (a backend
///   bug — caps are required to include at least `Codec::Vp9` per
///   [`MediaCapabilities::interim_default`]).
/// - In debug builds, if `max_width * max_height * 3 > 7 MiB`.
pub fn negotiate_session_config(peers: &[MediaCapabilities]) -> SessionVideoConfig {
    if peers.is_empty() {
        let caps = MediaCapabilities::interim_default();
        return config_from_single(&caps);
    }

    // Intersect supported codecs across all peers in the order of the
    // first peer's preference list. This produces deterministic output
    // regardless of which peer iterates first.
    let codec = pick_codec(peers);

    // Minimum resolution / fps across the mesh. width × height is
    // approximated from `max_pixel_count` assuming the 16:9 aspect ratio
    // that the WebView capture pipeline produces. The reverse map
    // (pixel_count → width × height) sticks to powers of two so the
    // encoder gets clean numbers (e.g. 1280×720, 854×480).
    let max_pixel_count = peers
        .iter()
        .map(|c| c.max_pixel_count)
        .min()
        .expect("non-empty checked above");
    let (mut max_width, mut max_height) = resolution_from_pixel_count(max_pixel_count);
    let max_fps = u32::from(
        peers
            .iter()
            .map(|c| c.max_fps)
            .min()
            .expect("non-empty checked above"),
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

    let scalability_mode = pick_scalability_mode(peers);
    let optimize_for_latency = peers.iter().all(|c| c.supports_optimize_for_latency);

    SessionVideoConfig {
        encoder: EncoderConstraints {
            codec,
            max_width,
            max_height,
            max_fps,
            scalability_mode,
        },
        decoder: DecoderConstraints {
            codec,
            optimize_for_latency,
        },
    }
}

/// Single-peer shortcut: return a config tuned to one peer's caps. Used
/// when `peers.is_empty()` so the negotiator never returns a degenerate
/// (zero-fps, zero-resolution) config.
fn config_from_single(caps: &MediaCapabilities) -> SessionVideoConfig {
    let codec = *caps
        .codecs
        .first()
        .expect("interim_default seeds Codec::Vp9");
    let (max_width, max_height) = resolution_from_pixel_count(caps.max_pixel_count);
    let scalability_mode = if caps
        .supported_scalability_modes
        .contains(&ScalabilityMode::L1T2)
    {
        ScalabilityMode::L1T2
    } else {
        ScalabilityMode::Flat
    };
    SessionVideoConfig {
        encoder: EncoderConstraints {
            codec,
            max_width,
            max_height,
            max_fps: u32::from(caps.max_fps),
            scalability_mode,
        },
        decoder: DecoderConstraints {
            codec,
            optimize_for_latency: caps.supports_optimize_for_latency,
        },
    }
}

/// Pick the first codec from peer 0's preference list that every other
/// peer also supports. Panic with a clear `unreachable!` if the
/// intersection is empty — this means a peer with no codecs joined a
/// video channel, which is a backend bug because every cap-broadcast
/// site uses `MediaCapabilities::interim_default()` as the floor.
fn pick_codec(peers: &[MediaCapabilities]) -> Codec {
    debug_assert!(
        peers.iter().all(|c| !c.codecs.is_empty()),
        "negotiate_session_config: peer reported empty codec list — backend bug"
    );
    let first = &peers[0];
    for candidate in &first.codecs {
        if peers.iter().all(|p| p.codecs.contains(candidate)) {
            return *candidate;
        }
    }
    unreachable!(
        "negotiate_session_config: codec intersection empty across {} peers — \
        backend invariant violated (every cap-broadcast site must seed \
        MediaCapabilities::interim_default which includes Codec::Vp9)",
        peers.len()
    )
}

/// `ScalabilityMode::L1T2` only when every peer lists it; otherwise
/// `Flat`. No asymmetric configuration — see plan §A.
fn pick_scalability_mode(peers: &[MediaCapabilities]) -> ScalabilityMode {
    let all_l1t2 = peers.iter().all(|c| {
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
        // Mac WKWebView style: 720p @ 30, VP9, L1T2 supported,
        // optimizeForLatency works.
        MediaCapabilities {
            max_pixel_count: 1280 * 720,
            max_fps: 30,
            codecs: vec![Codec::Vp9],
            supports_optimize_for_latency: true,
            supported_scalability_modes: vec![ScalabilityMode::Flat, ScalabilityMode::L1T2],
        }
    }

    fn pop_caps() -> MediaCapabilities {
        // Pop!_OS WebKitGTK style: 1080p @ 30, VP9, no L1T2,
        // optimizeForLatency probe fails (the acute defect in the plan).
        MediaCapabilities {
            max_pixel_count: 1920 * 1080,
            max_fps: 30,
            codecs: vec![Codec::Vp9],
            supports_optimize_for_latency: false,
            supported_scalability_modes: vec![ScalabilityMode::Flat],
        }
    }

    #[test]
    fn empty_peers_falls_back_to_interim_default() {
        let cfg = negotiate_session_config(&[]);
        // Interim default ladder rung: 854×480.
        assert_eq!(cfg.encoder.codec, Codec::Vp9);
        assert_eq!(cfg.decoder.codec, Codec::Vp9);
        assert_eq!(cfg.encoder.max_width, 854);
        assert_eq!(cfg.encoder.max_height, 480);
        assert_eq!(cfg.encoder.max_fps, 15);
        assert_eq!(cfg.encoder.scalability_mode, ScalabilityMode::Flat);
        assert!(!cfg.decoder.optimize_for_latency);
    }

    #[test]
    fn single_peer_uses_that_peer_caps() {
        let caps = mac_caps();
        let cfg = negotiate_session_config(std::slice::from_ref(&caps));
        // Largest ladder rung ≤ 1280×720 = 1280×720 itself.
        assert_eq!(cfg.encoder.max_width, 1280);
        assert_eq!(cfg.encoder.max_height, 720);
        assert_eq!(cfg.encoder.max_fps, 30);
        // Single peer supports L1T2 → picked.
        assert_eq!(cfg.encoder.scalability_mode, ScalabilityMode::L1T2);
        assert!(cfg.decoder.optimize_for_latency);
    }

    #[test]
    fn mixed_caps_picks_minimum_resolution_and_strict_intersection() {
        // Mac (1280×720, L1T2 yes, optimize yes) + Pop (1920×1080, L1T2
        // no, optimize no). Expect 1280×720, Flat, optimize=false.
        let cfg = negotiate_session_config(&[mac_caps(), pop_caps()]);
        assert_eq!(cfg.encoder.codec, Codec::Vp9);
        assert_eq!(cfg.encoder.max_width, 1280);
        assert_eq!(cfg.encoder.max_height, 720);
        assert_eq!(cfg.encoder.max_fps, 30);
        assert_eq!(
            cfg.encoder.scalability_mode,
            ScalabilityMode::Flat,
            "L1T2 must drop out when ANY peer doesn't list it"
        );
        assert!(
            !cfg.decoder.optimize_for_latency,
            "optimizeForLatency must drop out when ANY peer reports unsupported"
        );
    }

    #[test]
    fn all_peers_l1t2_keeps_l1t2() {
        let two_mac = vec![mac_caps(), mac_caps()];
        let cfg = negotiate_session_config(&two_mac);
        assert_eq!(cfg.encoder.scalability_mode, ScalabilityMode::L1T2);
        assert!(cfg.decoder.optimize_for_latency);
    }

    #[test]
    fn pixel_count_floor_clamps_to_smallest_ladder_rung() {
        // Tiny cap below the smallest ladder rung → derived 16:9
        // resolution, not 426×240. Sanity test the fallback path.
        let tiny = MediaCapabilities {
            max_pixel_count: 100,
            max_fps: 10,
            codecs: vec![Codec::Vp9],
            supports_optimize_for_latency: false,
            supported_scalability_modes: vec![ScalabilityMode::Flat],
        };
        let cfg = negotiate_session_config(std::slice::from_ref(&tiny));
        // Derived width × height should be ≤ 100.
        assert!(cfg.encoder.max_width * cfg.encoder.max_height <= 100);
        assert_eq!(cfg.encoder.max_fps, 10);
    }
}
