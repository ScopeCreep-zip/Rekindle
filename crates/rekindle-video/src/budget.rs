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

/// One AIMD step from receiver feedback:
/// - loss above ~5 % → multiplicative decrease (×0.85);
/// - clean window → additive-ish ramp (×1.10), capped at 1.5× the
///   receiver's measured delivered kbps.
///
/// The 1.5× cap is GCC's increase rule (`A_hat < 1.5 · R_hat`,
/// draft-ietf-rmcat-gcc): delivered throughput is bounded by our own
/// pacer rate, so any cap ≤ 1.0× measured makes recovery impossible —
/// the rate ratchets down and pins at the floor (observed live: 350 →
/// 100 kbps in 9 s, pinned for the rest of the session, ladder forced
/// to 2 fps). Growth must be allowed to EXCEED delivered to discover
/// headroom; loss is the overshoot signal that brings it back down.
/// The cap therefore limits growth only — it never forces decay below
/// the previous target (a stalled ack window measuring ~0 kbps must
/// not crater a clean stream; decrease is the loss branch's job).
///
/// `feedback_kbps` is the receiver's VIDEO-ONLY byte count — no voice
/// reserve is subtracted here (that would double-count voice, which
/// rides unpaced beside the video pacer and is protected by the loss
/// backoff when a route actually saturates).
///
/// Result is always within `[VIDEO_MIN_KBPS, VIDEO_MAX_KBPS]`.
#[must_use]
pub fn target_from_feedback(prev_kbps: u32, feedback_kbps: u32, loss_q8: u8) -> u32 {
    let next = if loss_q8 > LOSS_BACKOFF_Q8 {
        // ×0.85 in exact integer math.
        u32::try_from(u64::from(prev_kbps) * 85 / 100).unwrap_or(u32::MAX)
    } else {
        let ramped = u32::try_from(u64::from(prev_kbps) * 110 / 100).unwrap_or(u32::MAX);
        let cap = u32::try_from(u64::from(feedback_kbps) * 3 / 2).unwrap_or(u32::MAX);
        ramped.min(cap.max(prev_kbps))
    };
    next.clamp(VIDEO_MIN_KBPS, VIDEO_MAX_KBPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_decreases_target() {
        let next = target_from_feedback(400, 1_000, 50);
        assert!(next < 400, "loss must back off: {next}");
        assert_eq!(next, 340); // 400 × 0.85
    }

    #[test]
    fn clean_feedback_ramps() {
        let next = target_from_feedback(350, 1_000, 0);
        assert!(next > 350, "clean window must ramp: {next}");
        assert_eq!(next, 385); // 350 × 1.10, cap (1500) not binding
    }

    #[test]
    fn ramp_capped_at_gcc_factor_of_delivered() {
        // Receiver measures 300 kbps delivered — growth may reach but
        // not exceed 1.5× that (GCC A_hat < 1.5 · R_hat).
        let next = target_from_feedback(440, 300, 0);
        assert_eq!(next, 450); // min(484, 1.5 × 300)
    }

    #[test]
    fn pinned_floor_recovers_when_clean() {
        // Regression for the live death spiral: target at the floor,
        // receiver measuring exactly what the pacer let through. A
        // 1.0×-delivered cap held this at 100 forever; the GCC cap
        // lets a clean stream climb back out.
        let mut t = VIDEO_MIN_KBPS;
        let mut delivered = VIDEO_MIN_KBPS;
        for _ in 0..30 {
            t = target_from_feedback(t, delivered, 0);
            delivered = t; // pacer follows target; receiver measures it
        }
        assert_eq!(t, VIDEO_MAX_KBPS, "clean feedback must escape the floor");
    }

    #[test]
    fn stalled_ack_window_does_not_crater_a_clean_stream() {
        // A transport hiccup yields an ack of ~0 kbps with no loss
        // marked. The cap limits growth only — never forces decay.
        let next = target_from_feedback(800, 1, 0);
        assert_eq!(next, 800, "growth capped, no decay without loss");
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
