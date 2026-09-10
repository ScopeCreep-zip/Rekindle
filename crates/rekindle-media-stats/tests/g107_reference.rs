//! Validates the E-model against ITU-T G.107 itself.
//!
//! Every bitrate and FEC decision the voice send loop makes is now
//! derived from `r_factor` / `mos_from_r`. If those disagree with the
//! standard, the instrument is wrong and so is everything downstream of
//! it — and it would be wrong *quietly*, reporting plausible numbers
//! that steer the codec the wrong way. This pins them before the live
//! measurements start.
//!
//! The reference values here are computed from G.107's published
//! formulae, not from our implementation — that is what makes this a
//! validation rather than a change-detector. A snapshot of our own
//! output would pass just as happily with the arithmetic inverted.
//!
//! ## Scope, stated honestly
//!
//! `mos_from_r` **is** G.107 Annex B and is checked against the
//! standard's curve directly.
//!
//! `r_factor` is deliberately *not* pure G.107: it drops `Is` and `A`,
//! uses the linear approximation of `Id` above the delay knee, and adds
//! a burst weighting plus a jitter penalty the standard has no term
//! for. Those are documented engineering choices, so they cannot be
//! checked against a reference table. What is checked instead is the
//! set of properties any correct impairment model must have — a clean
//! link scores the clean-link value, impairments only ever subtract,
//! and clustered loss costs more than the same loss spread out.
//!
//! ## How to read the MOS numbers (matters for parity comparisons)
//!
//! G.107 is the **narrowband** E-model: R tops out at 100 and MOS at
//! 4.5, calibrated against 300–3400 Hz telephony. Opus at 48 kHz is a
//! fullband codec that exceeds narrowband quality outright — the
//! wideband extension (G.107.1) runs R up to 129 for exactly this
//! reason. Scoring Opus on the narrowband scale therefore
//! *under-reports* absolute quality, and our clean-link ceiling of 4.29
//! is a floor on what a listener would actually say.
//!
//! The consequence for parity work: comparing our MOS against a MOS
//! quoted for another service is meaningless unless both used the same
//! model. What is valid is comparing the **raw measurements** — loss,
//! discard, jitter, RTT, bitrate — and comparing our own MOS to itself
//! across a change. Relative movement is sound; the absolute number is
//! conservative by construction.

use rekindle_media_stats::{mos_from_r, r_factor, ReceptionMetrics};

/// G.107 Annex B: `MOS = 1 + 0.035·R + R·(R−60)·(100−R)·7×10⁻⁶`,
/// clamped to [1, 4.5]. Written out independently so the assertions
/// below compare against the standard rather than against ourselves.
fn g107_annex_b(r: f64) -> f64 {
    if r <= 0.0 {
        return 1.0;
    }
    if r >= 100.0 {
        return 4.5;
    }
    1.0 + 0.035 * r + r * (r - 60.0) * (100.0 - r) * 7.0e-6
}

fn clean() -> ReceptionMetrics {
    ReceptionMetrics {
        loss_rate_q8: 0,
        discard_rate_q8: 0,
        burst_density_q8: 0,
        gap_density_q8: 0,
        burst_duration_ms: 0,
        gap_duration_ms: 0,
        jitter_ms: 0,
        packets_expected: 1000,
        packets_received: 1000,
    }
}

/// Q8 encoding of a percentage, matching what `ReceptionTracker`
/// produces: `0..=255` maps to `0.0..=1.0`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 0..=255 immediately before the cast"
)]
fn pct_q8(pct: f64) -> u8 {
    ((pct / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8
}

#[test]
fn mos_matches_g107_annex_b_across_the_curve() {
    // Spot values spanning every user-satisfaction band the standard
    // names, plus the endpoints where the clamp meets the formula.
    for r in [
        0.0, 10.0, 20.0, 35.0, 50.0, 60.0, 70.0, 80.0, 90.0, 93.2, 99.0, 100.0,
    ] {
        let expected = g107_annex_b(r);
        let actual = mos_from_r(r);
        assert!(
            (actual - expected).abs() < 1e-9,
            "R={r}: MOS {actual} != G.107 Annex B {expected}"
        );
    }
}

#[test]
fn mos_hits_the_published_anchor_values() {
    // Hand-checked against the formula, so a sign slip or a transposed
    // coefficient inside the polynomial cannot pass by matching itself.
    for (r, expected) in [
        (100.0, 4.5),   // upper clamp, and the formula's own value there
        (93.2, 4.4093), // G.107's default reference R0
        (80.0, 4.024),  // "satisfied" band floor
        (70.0, 3.597),  // "some users dissatisfied"
        (60.0, 3.1),    // "many users dissatisfied"; the cubic term is 0
        (50.0, 2.575),  // "nearly all users dissatisfied"
        (0.0, 1.0),     // lower clamp
    ] {
        let actual = mos_from_r(r);
        assert!(
            (actual - expected).abs() < 5e-4,
            "R={r}: MOS {actual} != published {expected}"
        );
    }
}

#[test]
fn mos_is_continuous_at_both_clamps() {
    // The clamps must agree with the formula they replace, or the curve
    // steps at R=0 and R=100. G.107's polynomial is exactly 1.0 and 4.5
    // at those points, so a discontinuity means our guard is wrong.
    assert!((mos_from_r(0.001) - 1.0).abs() < 1e-3);
    assert!((mos_from_r(99.999) - 4.5).abs() < 1e-3);
}

#[test]
fn mos_rises_monotonically_with_r() {
    let mut prev = mos_from_r(0.0);
    let mut r = 0.5;
    while r <= 100.0 {
        let now = mos_from_r(r);
        assert!(now >= prev - 1e-12, "MOS fell between R<{r} and R={r}");
        prev = now;
        r += 0.5;
    }
}

#[test]
fn a_clean_link_scores_r0_minus_the_codec_impairment() {
    // No loss, no discard, no jitter, delay below the knee — and the
    // answer is still not R0.
    //
    // `Ie_eff` collapses to plain `Ie` at zero packet loss, and `Ie` is
    // the *codec's* equipment impairment factor: what you give up by
    // not being G.711. It is nonzero for every compressing codec, so a
    // flawless Opus call correctly scores below a flawless PCM one.
    // 93.2 − 5.0 = 88.2, MOS ≈ 4.29.
    //
    // Asserted explicitly because the alternative reading — "a perfect
    // link should be 93.2" — is the intuitive one and is wrong, and
    // someone will eventually try to "fix" the 5-point gap.
    let r = r_factor(&clean(), 0);
    assert!(
        (r - 88.2).abs() < 1e-9,
        "clean link scored {r}, expected R0 - Ie = 93.2 - 5.0 = 88.2"
    );
    assert!(
        (mos_from_r(r) - 4.2924).abs() < 5e-4,
        "clean-link MOS was {}",
        mos_from_r(r)
    );
    // Nothing else may subtract on a clean link: the whole 5-point gap
    // is Ie, so Id and the jitter penalty must both be exactly zero.
    assert!(
        (93.2 - r - 5.0).abs() < 1e-9,
        "something beyond Ie is penalising a clean link"
    );
}

#[test]
fn delay_below_the_knee_is_free_and_above_it_is_not() {
    // G.107 treats delay as harmless until the ~177ms knee. Our own
    // latency budget (250ms mouth-to-ear over a 3-hop route) sits above
    // it, so the penalty must actually engage there — otherwise the
    // budget looks free and the anonymity cost is invisible.
    let base = r_factor(&clean(), 0);
    assert!(
        (r_factor(&clean(), 100) - base).abs() < 1e-9,
        "100ms is free"
    );
    assert!(
        (r_factor(&clean(), 170) - base).abs() < 1e-9,
        "170ms is free"
    );
    assert!(
        r_factor(&clean(), 250) < base - 1.0,
        "250ms must cost something"
    );
    assert!(
        r_factor(&clean(), 400) < r_factor(&clean(), 250),
        "G.114's 400ms 'acceptable' ceiling must score worse than 250ms"
    );
}

#[test]
fn impairments_only_ever_subtract() {
    // Every term in R = R0 - Is - Id - Ie_eff + A that we implement is
    // a subtraction, so no measurement may ever raise the score above
    // the clean-link value. A sign error here would make a degrading
    // link report as improving and drive the bitrate the wrong way.
    let base = r_factor(&clean(), 0);
    for loss_pct in [0.5, 1.0, 5.0, 10.0, 25.0, 50.0, 100.0] {
        let mut m = clean();
        m.loss_rate_q8 = pct_q8(loss_pct);
        assert!(
            r_factor(&m, 0) <= base,
            "{loss_pct}% loss scored above a clean link"
        );
    }
    for jitter in [1u32, 10, 50, 200, 5000] {
        let mut m = clean();
        m.jitter_ms = jitter;
        assert!(
            r_factor(&m, 0) <= base,
            "{jitter}ms jitter scored above clean"
        );
    }
    let mut discarded = clean();
    discarded.discard_rate_q8 = pct_q8(10.0);
    assert!(discarded_is_penalised(&discarded, base));
}

fn discarded_is_penalised(m: &ReceptionMetrics, base: f64) -> bool {
    // A discard is as absent as a loss to the decoder, so it must cost
    // the same — the distinction exists to tell the operator which fix
    // to apply, not to excuse the packet.
    r_factor(m, 0) < base
}

#[test]
fn loss_degrades_monotonically() {
    let mut prev = f64::MAX;
    for loss_pct in [0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 40.0] {
        let mut m = clean();
        m.loss_rate_q8 = pct_q8(loss_pct);
        let r = r_factor(&m, 0);
        assert!(r <= prev, "R rose going from lower loss to {loss_pct}%");
        prev = r;
    }
}

#[test]
fn burst_loss_costs_more_than_the_same_loss_spread_out() {
    // The reason the burst/gap split is measured at all. G.113's Ie-eff
    // underestimates clustered loss: PLC hides one missing frame and
    // cannot hide a run of them, so 5% in bursts is audibly worse than
    // 5% sprinkled evenly. If this ever stops holding, the burst
    // accounting in `ReceptionTracker` is dead weight.
    let mut uniform = clean();
    uniform.loss_rate_q8 = pct_q8(5.0);
    uniform.burst_density_q8 = 0;

    let mut bursty = clean();
    bursty.loss_rate_q8 = pct_q8(5.0);
    bursty.burst_density_q8 = pct_q8(100.0);

    assert!(
        r_factor(&bursty, 0) < r_factor(&uniform, 0),
        "bursty {} should score below uniform {} at equal loss",
        r_factor(&bursty, 0),
        r_factor(&uniform, 0)
    );
}

#[test]
fn r_stays_inside_the_scale_under_absurd_input() {
    // A peer can report anything; a forged or broken report must not
    // produce an out-of-range R that then feeds the MOS polynomial
    // outside its domain.
    let mut worst = clean();
    worst.loss_rate_q8 = 255;
    worst.discard_rate_q8 = 255;
    worst.burst_density_q8 = 255;
    worst.jitter_ms = u32::MAX;
    let r = r_factor(&worst, u32::MAX);
    assert!((0.0..=100.0).contains(&r), "R out of scale: {r}");
    let mos = mos_from_r(r);
    assert!((1.0..=4.5).contains(&mos), "MOS out of scale: {mos}");
}

#[test]
fn the_bands_the_link_state_machine_uses_match_g107s_own() {
    // `LinkTracker` splits Good/Fair/Poor at R=80 and R=60. G.107's
    // user-satisfaction bands put 80 at the "satisfied" floor and 60 at
    // "many users dissatisfied" — so the thresholds are the standard's,
    // not arbitrary. Pinned as MOS so the mapping is legible: anything
    // called "good" is >= 4.0 MOS, anything "poor" is below 3.1.
    assert!(mos_from_r(80.0) >= 4.0, "the Good floor must be >= 4.0 MOS");
    assert!(mos_from_r(60.0) >= 3.0, "the Fair floor must be >= 3.0 MOS");
    assert!(mos_from_r(59.9) < 3.1, "below Fair must be under 3.1 MOS");
}
