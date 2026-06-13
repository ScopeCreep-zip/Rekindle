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

/// Ceiling — even clean feedback never ramps past this on routes. Sized
/// for sustained 480p15 over a 3-hop Veilid Safe route; the old 1200
/// assumed direct-UDP throughput and a transient clean ramp saturated
/// the egress the unpaced voice stream shares, starving audio.
pub const VIDEO_MAX_KBPS: u32 = 600;

/// Receiver loss above which the AIMD step backs off: 13/256 ≈ 5 %,
/// the classic "video must adapt" threshold.
const LOSS_BACKOFF_Q8: u8 = 13;

/// Static payload-share estimate (Q10) for the 4 KiB-fragment frame
/// mix (~0.625) — used until the pacer has measured a real window.
pub const START_PAYLOAD_SHARE_Q10: u32 = 640;

/// Scale receiver-measured PAYLOAD goodput up to the WIRE domain the
/// AIMD runs in. The receiver can only count reassembled frame bytes;
/// the sender knows its own measured payload share (data payload ÷
/// wire bytes incl. parity + per-fragment overhead). Without this
/// scaling the GCC growth cap compares wire-rate apples to
/// payload-rate oranges: at share ≈ 0.625 the cap (1.5 × goodput)
/// lands BELOW the current rate and the policy hard-freezes — the
/// libwebrtc `WithOverhead` lesson (estimate in wire units, convert
/// at the encoder boundary).
#[must_use]
pub fn wire_feedback_kbps(payload_kbps: u32, share_q10: u32) -> u32 {
    let s = u64::from(share_q10.clamp(256, 1024));
    u32::try_from(u64::from(payload_kbps) * 1024 / s).unwrap_or(u32::MAX)
}

/// The media-rate target to hand the ENCODER for a wire-domain pacer
/// target: encoder output × (1/share) ≈ wire demand, so the encoder
/// must aim at `wire × share` or it overproduces into the pacer queue
/// (delta expiry → phantom loss → death spiral).
#[must_use]
pub fn encoder_target_kbps(wire_kbps: u32, share_q10: u32) -> u32 {
    let s = u64::from(share_q10.clamp(256, 1024));
    u32::try_from(u64::from(wire_kbps) * s / 1024).unwrap_or(u32::MAX)
}

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
    fn ceiling_is_route_realistic() {
        // Guard: the ceiling is sized for a 3-hop Veilid Safe route, not
        // direct UDP. Raising it re-opens the egress-saturation that
        // starves the unpaced voice stream — change deliberately.
        assert_eq!(VIDEO_MAX_KBPS, 600);
    }

    #[test]
    fn accurate_loss_backs_off_below_ceiling() {
        // With Part B feeding a real (non-zero) loss signal, the AIMD
        // backs off near the ceiling instead of pinning at it: 20/256 ≈
        // 8 % > the 5 % threshold → ×0.85.
        let next = target_from_feedback(580, 600, 20);
        assert!(next < 580, "real loss must back off near the ceiling: {next}");
        assert_eq!(next, 493); // 580 × 0.85
    }

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
        // (500 sits below the 600 ceiling so the clamp isn't what holds
        // it — the no-decay cap is.)
        let next = target_from_feedback(500, 1, 0);
        assert_eq!(next, 500, "growth capped, no decay without loss");
    }

    #[test]
    fn wire_domain_loop_converges_with_realistic_share() {
        // The R4 closed loop: encoder targets wire×share, receiver
        // measures that payload rate, sender scales it back to wire
        // before the AIMD step. Must reach the ceiling from the start
        // value on a clean link.
        let share = START_PAYLOAD_SHARE_Q10;
        let mut t = VIDEO_START_KBPS;
        for _ in 0..30 {
            let payload_goodput = encoder_target_kbps(t, share);
            let wire_feedback = wire_feedback_kbps(payload_goodput, share);
            t = target_from_feedback(t, wire_feedback, 0);
        }
        assert_eq!(t, VIDEO_MAX_KBPS, "clean wire-domain loop must reach ceiling");
    }

    #[test]
    fn payload_domain_feedback_would_freeze_documenting_the_bug() {
        // Control test for the fix above: feeding PAYLOAD goodput
        // straight into the wire-domain AIMD freezes the target —
        // 1.5 × 0.625 < 1.0 lands the cap below prev.
        let payload_goodput = encoder_target_kbps(350, START_PAYLOAD_SHARE_Q10); // 218
        assert_eq!(target_from_feedback(350, payload_goodput, 0), 350);
    }

    #[test]
    fn unit_conversions_round_trip_and_clamp() {
        for share in [256u32, 640, 1024] {
            let wire = 800u32;
            let media = encoder_target_kbps(wire, share);
            let back = wire_feedback_kbps(media, share);
            assert!(
                back.abs_diff(wire) <= 4,
                "share {share}: {wire} → {media} → {back} drifted"
            );
        }
        // Out-of-range shares clamp instead of zeroing/exploding.
        assert_eq!(encoder_target_kbps(1000, 0), encoder_target_kbps(1000, 256));
        assert_eq!(wire_feedback_kbps(1000, 4096), wire_feedback_kbps(1000, 1024));
        // The stalled-ack guard survives the wire scaling: ~0 feedback
        // never decays a clean stream. (500 < the 600 ceiling so it's the
        // no-decay cap holding, not the clamp.)
        assert_eq!(target_from_feedback(500, wire_feedback_kbps(1, 640), 0), 500);
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
