//! Turning measurements into a verdict — the ITU-T G.107 E-model, and
//! the link state a call actually acts on.
//!
//! [`ReceptionMetrics`](crate::ReceptionMetrics) says what happened.
//! This says what it *means*: one score a user can read, and one state
//! the session can branch on.

use serde::{Deserialize, Serialize};

use crate::ReceptionMetrics;

/// ITU-T G.107 default R for a clean narrowband link, before any
/// impairment is subtracted. 93.2 is the standard's own starting point.
const R0_CLEAN: f64 = 93.2;

/// Delay above which G.107's `Id` term starts to bite, milliseconds.
/// Below this, one-way delay is imperceptible in conversation.
const DELAY_KNEE_MS: f64 = 177.3;

/// Opus packet-loss robustness factor (`Bpl` in G.113's Ie-eff term).
///
/// G.113 tabulates this per codec; Opus is not in the original table,
/// so this uses the value commonly fitted for it — appreciably more
/// loss-tolerant than G.711 (`Bpl` ≈ 4.3) because of in-band FEC and
/// PLC. Higher means loss hurts less.
const OPUS_BPL: f64 = 18.0;

/// Opus equipment impairment at zero loss (`Ie`). Near-transparent for
/// speech at our bitrates.
const OPUS_IE: f64 = 5.0;

/// How a link is behaving, and what the session should do about it.
///
/// A state machine rather than a threshold because the useful question
/// is not "is loss above 5 %" but "has this link stopped working, and
/// has it come back" — and answering that needs memory and hysteresis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    /// Usable. Nothing to say to the user.
    Good,
    /// Degraded but carrying media — worth showing, not worth acting on.
    Fair,
    /// Bad enough that the user should be told, and the sender should
    /// back off.
    Poor,
    /// Nothing is arriving. The session is held open and probing.
    ///
    /// Deliberately not "ended": a call that drops for four seconds in
    /// a lift should resume, not require redialling.
    Lost,
    /// Media has started arriving again after [`Self::Lost`].
    ///
    /// The distinct state exists so recovery can be *acted on* — a
    /// video sender must send a keyframe here, because the peer's
    /// decoder has a hole in its reference chain and will otherwise
    /// show garbage until the next scheduled keyframe.
    Recovering,
}

impl LinkState {
    /// Whether media is flowing at all.
    #[must_use]
    pub fn is_connected(self) -> bool {
        !matches!(self, Self::Lost)
    }

    /// Whether a video sender should force a keyframe now.
    #[must_use]
    pub fn needs_keyframe(self) -> bool {
        matches!(self, Self::Recovering)
    }
}

/// A quality verdict for one stream.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityScore {
    /// G.107 R factor, 0–100. 94 is toll quality; below 50 is
    /// "not recommended".
    pub r_factor: u8,
    /// Listening-quality MOS, 1.0–5.0 — ignores delay, so it answers
    /// "does the audio sound clean".
    pub mos_lq: f32,
    /// Conversational-quality MOS, 1.0–5.0 — includes delay, so it
    /// answers "can you hold a conversation over it". A satellite link
    /// can be flawless on `mos_lq` and unusable on `mos_cq`.
    pub mos_cq: f32,
    pub state: LinkState,
}

/// Compute an R factor from measurements.
///
/// Follows the G.107 shape — `R = R0 - Is - Id - Ie_eff + A` — with the
/// terms we can actually observe. `Is` (signal impairment) and `A`
/// (advantage factor) are dropped: the first needs analogue levels we
/// do not measure, and the second is a policy fudge for tolerating
/// worse quality on mobile, which would only flatter us here.
///
/// **Burst loss is weighted more heavily than uniform loss**, which is
/// the whole reason the burst/gap split is measured — G.113's Ie-eff
/// underestimates damage when loss is clustered.
#[must_use]
pub fn r_factor(metrics: &ReceptionMetrics, one_way_delay_ms: u32) -> f64 {
    let loss = q8_to_fraction(metrics.loss_rate_q8);
    let discard = q8_to_fraction(metrics.discard_rate_q8);
    // A discarded packet is as absent as a lost one to the decoder.
    let total_loss_pct = (loss + discard) * 100.0;

    // Clustered loss does more damage than the same amount spread out:
    // PLC interpolates a single missing frame convincingly and cannot
    // paper over a run of them. Scale effective loss by how dense the
    // bursts are, up to 1.5x.
    let burst_weight = 1.0 + 0.5 * q8_to_fraction(metrics.burst_density_q8);
    let effective_loss = total_loss_pct * burst_weight;

    // G.113 Ie-eff: Ie + (95 - Ie) * Ppl / (Ppl/BurstR + Bpl)
    let ie_eff = OPUS_IE + (95.0 - OPUS_IE) * effective_loss / (effective_loss + OPUS_BPL);

    // G.107 Id: delay is free below the knee, then costs steadily.
    let delay = f64::from(one_way_delay_ms);
    let id = if delay < DELAY_KNEE_MS {
        0.0
    } else {
        // The standard's Idd curve is piecewise and awkward; this is the
        // widely used linear approximation above the knee, which tracks
        // it closely through the range a call can survive.
        0.024 * delay + 0.11 * (delay - DELAY_KNEE_MS)
    };

    // Jitter that survives the buffer is heard as choppiness, and G.107
    // has no term for it — the buffer is supposed to absorb it. What
    // leaks through shows up as discards, already counted above; this
    // small extra penalty accounts for the buffer growing to cope,
    // which costs delay the caller may not have told us about.
    let jitter_penalty = f64::from(metrics.jitter_ms).min(100.0) * 0.05;

    (R0_CLEAN - id - ie_eff - jitter_penalty).clamp(0.0, 100.0)
}

/// G.107 Annex B: R → MOS.
#[must_use]
pub fn mos_from_r(r: f64) -> f64 {
    if r <= 0.0 {
        return 1.0;
    }
    if r >= 100.0 {
        return 4.5;
    }
    let mos = 1.0 + 0.035 * r + r * (r - 60.0) * (100.0 - r) * 7.0e-6;
    mos.clamp(1.0, 4.5)
}

fn q8_to_fraction(q8: u8) -> f64 {
    f64::from(q8) / 255.0
}

/// Tracks [`LinkState`] across successive reports, with the hysteresis
/// that stops a call flapping between "poor" and "good" on one bad
/// window.
#[derive(Debug, Clone)]
pub struct LinkTracker {
    state: LinkState,
    /// Consecutive reports with no packets — the loss threshold.
    silent_reports: u32,
    /// Reports remaining before `Recovering` settles to a live state.
    recovery_hold: u32,
    silence_limit: u32,
    recovery_reports: u32,
}

impl Default for LinkTracker {
    fn default() -> Self {
        Self::new(3, 2)
    }
}

impl LinkTracker {
    /// `silence_limit` — consecutive empty reports before declaring the
    /// link lost. `recovery_reports` — reports of returning media
    /// before leaving `Recovering`.
    #[must_use]
    pub fn new(silence_limit: u32, recovery_reports: u32) -> Self {
        Self {
            state: LinkState::Good,
            silent_reports: 0,
            recovery_hold: 0,
            silence_limit: silence_limit.max(1),
            recovery_reports: recovery_reports.max(1),
        }
    }

    #[must_use]
    pub fn state(&self) -> LinkState {
        self.state
    }

    /// Fold in one report window and return the new state.
    ///
    /// `packets_this_window` is what arrived since the last call — the
    /// silence signal. Rates alone cannot detect a dead link: a stream
    /// delivering nothing has no packets to compute a loss rate from,
    /// and would otherwise read as a clean 0 %.
    pub fn observe(&mut self, packets_this_window: u64, r: f64) -> LinkState {
        if packets_this_window == 0 {
            self.silent_reports += 1;
            self.recovery_hold = 0;
            if self.silent_reports >= self.silence_limit {
                self.state = LinkState::Lost;
            }
            return self.state;
        }

        // Media is arriving.
        let was_lost = matches!(self.state, LinkState::Lost);
        self.silent_reports = 0;

        if was_lost {
            self.state = LinkState::Recovering;
            self.recovery_hold = self.recovery_reports;
            return self.state;
        }

        if self.recovery_hold > 0 {
            self.recovery_hold -= 1;
            if self.recovery_hold > 0 {
                return self.state;
            }
        }

        // G.107's own bands: 90+ best, 80+ high, 70+ medium, 60+ low.
        self.state = if r >= 80.0 {
            LinkState::Good
        } else if r >= 60.0 {
            LinkState::Fair
        } else {
            LinkState::Poor
        };
        self.state
    }
}

/// Score a stream and advance its link state in one step.
/// MOS is reported as `f32`: it is a 1.0–4.5 opinion score shown to a
/// human, where `f64` precision is meaningless, and `f32` keeps the
/// event payload small on the wire.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "r is clamped to 0..=100; MOS is a display value in 1.0..=4.5"
)]
#[must_use]
pub fn score(
    metrics: &ReceptionMetrics,
    one_way_delay_ms: u32,
    packets_this_window: u64,
    tracker: &mut LinkTracker,
) -> QualityScore {
    let r = r_factor(metrics, one_way_delay_ms);
    let state = tracker.observe(packets_this_window, r);
    QualityScore {
        // `r` is clamped to 0..=100 by `r_factor`, so the cast is
        // in range; `saturating` states that rather than relying on it.
        r_factor: r.round().clamp(0.0, 100.0) as u8,
        mos_lq: mos_from_r(r_factor(metrics, 0)) as f32,
        mos_cq: mos_from_r(r) as f32,
        state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> ReceptionMetrics {
        ReceptionMetrics {
            packets_expected: 1000,
            packets_received: 1000,
            ..Default::default()
        }
    }

    #[test]
    fn a_clean_link_scores_near_toll_quality() {
        let r = r_factor(&clean(), 40);
        assert!(r > 85.0, "clean link should be high, got {r}");
        assert!(mos_from_r(r) > 4.0, "and sound good");
    }

    #[test]
    fn loss_lowers_the_score() {
        let mut lossy = clean();
        lossy.loss_rate_q8 = 26; // ~10 %
        assert!(r_factor(&lossy, 40) < r_factor(&clean(), 40));
    }

    /// The reason burst/gap is measured at all.
    #[test]
    fn clustered_loss_scores_worse_than_spread_loss_at_equal_rate() {
        let mut spread = clean();
        spread.loss_rate_q8 = 13; // ~5 %
        spread.burst_density_q8 = 10; // mostly isolated

        let mut clustered = clean();
        clustered.loss_rate_q8 = 13; // the same ~5 %
        clustered.burst_density_q8 = 200; // arriving in dense runs

        assert!(
            r_factor(&clustered, 40) < r_factor(&spread, 40),
            "identical loss rates must not score identically when one is bursty"
        );
    }

    #[test]
    fn discards_hurt_as_much_as_loss() {
        let mut lost = clean();
        lost.loss_rate_q8 = 26;
        let mut discarded = clean();
        discarded.discard_rate_q8 = 26;
        // A packet that arrived too late is as absent as one that never came.
        let a = r_factor(&lost, 40);
        let b = r_factor(&discarded, 40);
        assert!((a - b).abs() < 0.01, "{a} vs {b}");
    }

    #[test]
    fn delay_separates_listening_from_conversational_quality() {
        let m = clean();
        // A satellite-class one-way delay: nothing is lost, so it still
        // *sounds* perfect, but you cannot converse over it.
        let listening = mos_from_r(r_factor(&m, 0));
        let conversational = mos_from_r(r_factor(&m, 600));
        assert!(
            listening > conversational + 0.5,
            "delay must move CQ without moving LQ: {listening} vs {conversational}"
        );
    }

    #[test]
    fn silence_declares_the_link_lost_then_recovers() {
        let mut t = LinkTracker::new(3, 2);
        assert_eq!(t.observe(100, 90.0), LinkState::Good);

        // Three empty windows.
        assert_ne!(t.observe(0, 0.0), LinkState::Lost, "one is not enough");
        assert_ne!(t.observe(0, 0.0), LinkState::Lost, "nor two");
        assert_eq!(t.observe(0, 0.0), LinkState::Lost, "three is");
        assert!(!t.state().is_connected());

        // Media returns.
        let s = t.observe(100, 90.0);
        assert_eq!(s, LinkState::Recovering);
        assert!(
            s.needs_keyframe(),
            "the peer's decoder has a hole; it needs an intra frame"
        );
        assert!(s.is_connected());

        // And settles.
        t.observe(100, 90.0);
        assert_eq!(t.observe(100, 90.0), LinkState::Good);
    }

    #[test]
    fn a_dead_link_is_not_mistaken_for_a_clean_one() {
        // Nothing arriving means no loss *rate* to compute — the trap
        // this guards. Silence must be detected by packet count.
        let mut t = LinkTracker::new(2, 1);
        t.observe(0, 100.0);
        assert_eq!(
            t.observe(0, 100.0),
            LinkState::Lost,
            "a perfect R factor over zero packets is silence, not quality"
        );
    }
}
