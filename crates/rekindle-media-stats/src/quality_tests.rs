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
