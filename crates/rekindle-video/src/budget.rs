//! Audio-first bandwidth budget — the libwebrtc `BitrateAllocator`
//! analog for media over Veilid private routes.
//!
//! Voice gets a fixed reserve FIRST; video gets what remains of the
//! congestion estimate, clamped to what multi-hop Veilid routes
//! realistically sustain. The feedback step is AIMD (additive-increase
//! ramp, multiplicative decrease on loss) — the same shape GCC uses —
//! driven by the receiver `FrameAck` kbps/loss measurements.
//!
//! Pure functions; the adapter (`video_adapter::emit_event`) owns the
//! per-(community, channel) previous-target state.

/// Absolute audio reserve. Opus at 32 kbps plus envelope/signature
/// overhead lands ~70-80 kbps on the wire; 64 kbps of *budget headroom*
/// is reserved out of the video estimate so video can never price
/// voice out (voice itself is structurally unpaced — see `pacer.rs`).
pub const VOICE_RESERVE_KBPS: u32 = 64;

/// Starting video target: 480p15 over multi-hop Veilid routes. The
/// previous 800 kbps default assumed direct-UDP-class throughput and
/// saturated routes hard enough to drop voice.
pub const VIDEO_START_KBPS: u32 = 350;

/// Floor below which video is not worth sending (the encoder smears
/// 480p into mud); the AIMD step never goes lower — sustained loss at
/// the floor is a route problem, not a bitrate problem.
pub const VIDEO_MIN_KBPS: u32 = 100;

/// Ceiling — even clean feedback never ramps past this on routes.
pub const VIDEO_MAX_KBPS: u32 = 1200;

/// Receiver loss above which the AIMD step backs off: 13/256 ≈ 5 %,
/// the classic "video must adapt" threshold.
const LOSS_BACKOFF_Q8: u8 = 13;

/// The video budget for a given downstream estimate: the estimate
/// minus the voice reserve, clamped to `[MIN, MAX]`. `None` (no
/// feedback yet) → the conservative start value.
#[must_use]
pub fn video_budget_kbps(estimate_kbps: Option<u32>) -> u32 {
    match estimate_kbps {
        None => VIDEO_START_KBPS,
        Some(est) => est
            .saturating_sub(VOICE_RESERVE_KBPS)
            .clamp(VIDEO_MIN_KBPS, VIDEO_MAX_KBPS),
    }
}

/// One AIMD step from receiver feedback:
/// - loss above ~5 % → multiplicative decrease (×0.85);
/// - clean window → additive-ish ramp (×1.10), capped by the budget
///   derived from the receiver's own kbps estimate.
///
/// Result is always within `[VIDEO_MIN_KBPS, VIDEO_MAX_KBPS]`.
#[must_use]
pub fn target_from_feedback(prev_kbps: u32, feedback_kbps: u32, loss_q8: u8) -> u32 {
    let next = if loss_q8 > LOSS_BACKOFF_Q8 {
        // ×0.85 in exact integer math.
        u32::try_from(u64::from(prev_kbps) * 85 / 100).unwrap_or(u32::MAX)
    } else {
        // ×1.10, capped by the receiver-derived budget.
        let ramped = u32::try_from(u64::from(prev_kbps) * 110 / 100).unwrap_or(u32::MAX);
        ramped.min(video_budget_kbps(Some(feedback_kbps)))
    };
    next.clamp(VIDEO_MIN_KBPS, VIDEO_MAX_KBPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_clamps_and_defaults() {
        assert_eq!(video_budget_kbps(None), VIDEO_START_KBPS);
        // Estimate below the reserve → floor, never zero/underflow.
        assert_eq!(video_budget_kbps(Some(40)), VIDEO_MIN_KBPS);
        assert_eq!(video_budget_kbps(Some(0)), VIDEO_MIN_KBPS);
        // Mid-range: reserve comes off the top.
        assert_eq!(video_budget_kbps(Some(500)), 500 - VOICE_RESERVE_KBPS);
        // Huge estimate → ceiling.
        assert_eq!(video_budget_kbps(Some(100_000)), VIDEO_MAX_KBPS);
    }

    #[test]
    fn loss_decreases_target() {
        let next = target_from_feedback(400, 1_000, 50);
        assert!(next < 400, "loss must back off: {next}");
        assert_eq!(next, 340); // 400 × 0.85
    }

    #[test]
    fn clean_feedback_ramps_toward_budget() {
        let next = target_from_feedback(350, 1_000, 0);
        assert!(next > 350, "clean window must ramp: {next}");
        assert_eq!(next, 385); // 350 × 1.10, budget (936) not binding
    }

    #[test]
    fn ramp_capped_by_receiver_estimate() {
        // Receiver only measures 300 kbps downstream — the ramp may
        // never exceed (300 − reserve).
        let next = target_from_feedback(350, 300, 0);
        assert_eq!(next, 300 - VOICE_RESERVE_KBPS);
    }

    #[test]
    fn clamps_hold_under_extremes() {
        // Sustained loss can't go below the floor…
        let mut t = VIDEO_MIN_KBPS;
        for _ in 0..50 {
            t = target_from_feedback(t, 1_000, 255);
        }
        assert_eq!(t, VIDEO_MIN_KBPS);
        // …and a clean firehose can't exceed the ceiling.
        let mut t = VIDEO_MAX_KBPS;
        for _ in 0..50 {
            t = target_from_feedback(t, 1_000_000, 0);
        }
        assert_eq!(t, VIDEO_MAX_KBPS);
    }
}
