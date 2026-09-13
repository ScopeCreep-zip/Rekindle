//! Capture-format negotiation: libwebrtc's `GetBestMatchedCapability`
//! (`modules/video_capture/device_info_impl.cc`) over the camera's real
//! enumerated modes.
//!
//! The camera is asked for a NATIVE mode near the encode target, never for
//! the encode target itself — `RequestedFormatType::Closest(854×480, NV12)`
//! fails on macOS because 854 is never a sensor width and nokhwa's `Closest`
//! filters by exact `FrameFormat` before distance, returning `None` when
//! that filter empties. Here we enumerate → score → select a real mode;
//! `convert.rs`/`scale.rs` then bring it DOWN to the encode target.
//!
//! Precedence is strictly lexicographic — **height, then width, then frame
//! rate, then pixel format**. On each ordered dimension a candidate is
//! "better" when it meets-or-exceeds the target with the least overshoot,
//! or (when every mode is below target) is the largest — libwebrtc's
//! `(diff >= 0 && diff <= |best_diff|) || (best_diff < 0 && diff >= best_diff)`.
//! Pixel format breaks a full geometry+fps tie, preferring raw formats over
//! per-frame-decoded MJPEG.

use nokhwa::utils::{CameraFormat, FrameFormat};

/// One ordered dimension's verdict comparing a candidate's `diff`
/// (`candidate - target`) against the running best's `diff`.
enum DimCmp {
    /// The candidate is closer to the target (or overshoots by less) on this
    /// dimension — it wins outright.
    Better,
    /// Identical diff — defer to the next-lower dimension.
    Tie,
    /// The running best is closer; the candidate loses here.
    Worse,
}

/// libwebrtc's per-dimension preference: meet-or-exceed the target with the
/// least overshoot; if BOTH are below target, the larger (least-negative)
/// value wins. Expressed as the exact predicate
/// `(d >= 0 && d <= |b|) || (b < 0 && d >= b)` for "candidate at least as
/// good as best", then split into strictly-better vs an exact tie. Diffs are
/// `i64` (widened losslessly from `u32`) so no cast can wrap or truncate.
fn cmp_dim(cand_diff: i64, best_diff: i64) -> DimCmp {
    if cand_diff == best_diff {
        return DimCmp::Tie;
    }
    let cand_at_least_as_good = (cand_diff >= 0 && cand_diff <= best_diff.abs())
        || (best_diff < 0 && cand_diff >= best_diff);
    if cand_at_least_as_good {
        DimCmp::Better
    } else {
        DimCmp::Worse
    }
}

/// Pixel-format tiebreak rank (lower = preferred), applied ONLY when height,
/// width and frame rate all tie. Raw formats beat compressed: NV12 and YUYV
/// are the encoder-friendly repacks (`convert.rs` fast paths); RAWRGB/RAWBGR
/// are raw packed RGB; GRAY is raw but grayscale (poor for a colour call, so
/// just above compressed); MJPEG is LAST — a per-frame JPEG decode. Every
/// nokhwa `FrameFormat` gets a rank so selection is a total, deterministic
/// order.
fn format_rank(format: FrameFormat) -> u8 {
    match format {
        FrameFormat::NV12 => 0,
        FrameFormat::YUYV => 1,
        FrameFormat::RAWRGB => 2,
        FrameFormat::RAWBGR => 3,
        FrameFormat::GRAY => 4,
        FrameFormat::MJPEG => 5,
    }
}

/// True when `candidate` should replace `best` under the lexicographic rule:
/// height, then (only on a height tie) width, then (only on a height+width
/// tie) frame rate, then (only on a full geometry+fps tie) pixel format.
fn is_better(
    candidate: &CameraFormat,
    best: &CameraFormat,
    target_w: u32,
    target_h: u32,
    target_fps: u32,
) -> bool {
    // Height first.
    match cmp_dim(
        i64::from(candidate.height()) - i64::from(target_h),
        i64::from(best.height()) - i64::from(target_h),
    ) {
        DimCmp::Better => return true,
        DimCmp::Worse => return false,
        DimCmp::Tie => {}
    }
    // Width only when height ties.
    match cmp_dim(
        i64::from(candidate.width()) - i64::from(target_w),
        i64::from(best.width()) - i64::from(target_w),
    ) {
        DimCmp::Better => return true,
        DimCmp::Worse => return false,
        DimCmp::Tie => {}
    }
    // Frame rate only when height AND width tie.
    match cmp_dim(
        i64::from(candidate.frame_rate()) - i64::from(target_fps),
        i64::from(best.frame_rate()) - i64::from(target_fps),
    ) {
        DimCmp::Better => return true,
        DimCmp::Worse => return false,
        DimCmp::Tie => {}
    }
    // Full geometry+fps tie — pixel format is the final tiebreak.
    format_rank(candidate.format()) < format_rank(best.format())
}

/// Select the device mode best matching the encode target via libwebrtc's
/// `GetBestMatchedCapability`. Returns `None` only when `available` is empty.
///
/// The fold reproduces libwebrtc's imperative loop: the first mode seeds the
/// running best, each later mode replaces it iff [`is_better`] holds. Because
/// `is_better` is a strict lexicographic order, the result is independent of
/// the order `available` is listed in.
pub(crate) fn select_capture_format(
    available: &[CameraFormat],
    target_w: u32,
    target_h: u32,
    target_fps: u32,
) -> Option<CameraFormat> {
    let mut best = *available.first()?;
    for candidate in available.iter().skip(1) {
        if is_better(candidate, &best, target_w, target_h, target_fps) {
            best = *candidate;
        }
    }
    Some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(w: u32, h: u32, fps: u32, format: FrameFormat) -> CameraFormat {
        CameraFormat::new_from(w, h, format, fps)
    }

    #[test]
    fn select_prefers_exact_height_match_over_taller_modes() {
        // Target 854×480@15. Candidates by height: 480, 720, 1080 (all NV12,
        // 30 fps). Height is the FIRST lexicographic key, and 640×480 matches
        // the target height 480 EXACTLY (zero overshoot) — the optimum on
        // height. The taller modes overshoot height and are eliminated before
        // width is ever considered, so 640×480 wins outright even though its
        // width (640) is BELOW the 854 target. This is faithful libwebrtc
        // behaviour (height dominates width), NOT the naive "smallest mode ≥
        // target in both dimensions" pick (which would be 1280×720);
        // convert/scale handle the sub-target width downstream. Working the
        // rule: for 1280×720 vs best 640×480, diffH=240, bestDiffH=0 →
        // (240>=0 && 240<=|0|) is false and (0<0 …) is false, so 720 is not
        // "at least as good" and loses on height.
        let available = [
            fmt(1280, 720, 30, FrameFormat::NV12),
            fmt(640, 480, 30, FrameFormat::NV12),
            fmt(1920, 1080, 30, FrameFormat::NV12),
        ];
        let best = select_capture_format(&available, 854, 480, 15).unwrap();
        assert_eq!((best.width(), best.height()), (640, 480));
    }

    #[test]
    fn select_all_below_target_picks_largest() {
        // Every mode is below a 1920×1080 target height; the least-negative
        // (largest) height wins — 1280×720 over 640×480. Working the rule:
        // for 1280×720 vs best 640×480, diffH=-360, bestDiffH=-600 →
        // (bestDiffH<0 && -360>=-600) is true, so 720 is strictly better.
        let available = [
            fmt(640, 480, 30, FrameFormat::NV12),
            fmt(1280, 720, 30, FrameFormat::NV12),
        ];
        let best = select_capture_format(&available, 1920, 1080, 30).unwrap();
        assert_eq!((best.width(), best.height()), (1280, 720));
    }

    #[test]
    fn select_width_tiebreak_when_height_matches() {
        // All heights equal the target (480) → height ties, so width decides.
        // 640 matches the target width exactly; 800 overshoots, 320 is below
        // — 640 wins. Proves width is compared only after a height tie.
        let available = [
            fmt(800, 480, 30, FrameFormat::NV12),
            fmt(320, 480, 30, FrameFormat::NV12),
            fmt(640, 480, 30, FrameFormat::NV12),
        ];
        let best = select_capture_format(&available, 640, 480, 30).unwrap();
        assert_eq!((best.width(), best.height()), (640, 480));
    }

    #[test]
    fn select_fps_tiebreak_when_geometry_matches() {
        // Geometry ties at the target → fps decides: 30 meets the 30 target
        // exactly (zero overshoot), beating 60 (overshoot) and 15 (below).
        // Proves fps is compared only after height AND width tie.
        let available = [
            fmt(1280, 720, 15, FrameFormat::NV12),
            fmt(1280, 720, 60, FrameFormat::NV12),
            fmt(1280, 720, 30, FrameFormat::NV12),
        ];
        let best = select_capture_format(&available, 1280, 720, 30).unwrap();
        assert_eq!(best.frame_rate(), 30);
    }

    #[test]
    fn select_exact_geometry_tie_prefers_raw_over_mjpeg() {
        // Identical geometry+fps → pixel format is the final tiebreak.
        // NV12 (raw) beats YUYV (raw) beats MJPEG (compressed, per-frame
        // decode). MJPEG is listed FIRST to prove the result is order-
        // independent, not an artefact of the seed.
        let available = [
            fmt(1280, 720, 30, FrameFormat::MJPEG),
            fmt(1280, 720, 30, FrameFormat::YUYV),
            fmt(1280, 720, 30, FrameFormat::NV12),
        ];
        let best = select_capture_format(&available, 1280, 720, 30).unwrap();
        assert_eq!(best.format(), FrameFormat::NV12);

        // Raw YUYV still beats compressed MJPEG when NV12 is absent.
        let two = [
            fmt(1280, 720, 30, FrameFormat::MJPEG),
            fmt(1280, 720, 30, FrameFormat::YUYV),
        ];
        assert_eq!(
            select_capture_format(&two, 1280, 720, 30).unwrap().format(),
            FrameFormat::YUYV
        );
    }

    #[test]
    fn select_empty_is_none() {
        assert!(select_capture_format(&[], 1280, 720, 30).is_none());
    }
}
