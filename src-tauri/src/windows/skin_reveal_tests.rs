use super::{Show, SkinRevealEffect, SkinRevealGate, LOAD_LIMIT};
use std::time::Instant;

#[test]
fn matching_commit_and_shape_reveal_the_first_skinned_window() {
    let now = Instant::now();
    let mut gate = SkinRevealGate::default();
    gate.request(now, Show::First);
    gate.commit("skin://old".to_owned(), now);
    gate.allocate(Some("skin://current"), now);
    assert_eq!(gate.poll(now), SkinRevealEffect::Wait);
    gate.commit("skin://current".to_owned(), now);
    assert_eq!(gate.poll(now), SkinRevealEffect::Skinned(Show::First));
    assert_eq!(gate.poll(now + LOAD_LIMIT), SkinRevealEffect::Wait);
}

#[test]
fn deadline_reveals_and_reports_even_without_webview_activity() {
    let now = Instant::now();
    let mut gate = SkinRevealGate::default();
    let end = gate
        .request(now, Show::First)
        .expect("first reveal arms the deadline");
    assert_eq!(end, now + LOAD_LIMIT);
    assert_eq!(gate.poll(now), SkinRevealEffect::Wait);
    assert_eq!(
        gate.poll(end),
        SkinRevealEffect::RevealAndReportTimeout(Show::First)
    );
    gate.allocate(Some("skin://late"), now);
    gate.commit("skin://late".to_owned(), now);
    assert_eq!(gate.poll(end), SkinRevealEffect::Wait);
    gate.request(end, Show::Raise);
    assert_eq!(gate.poll(end), SkinRevealEffect::Failed(Show::Raise));
}

#[test]
fn reopen_waits_for_the_current_allocation_and_commit_under_the_same_deadline() {
    let now = Instant::now();
    let mut gate = SkinRevealGate::default();
    gate.request(now, Show::First);
    gate.allocate(Some("skin://old"), now);
    gate.commit("skin://old".to_owned(), now);
    assert_eq!(gate.poll(now), SkinRevealEffect::Skinned(Show::First));

    gate.allocate(None, now);
    let end = gate
        .request(now, Show::Raise)
        .expect("reopen arms a deadline");
    gate.allocate(Some("skin://new"), now);
    assert_eq!(gate.poll(now), SkinRevealEffect::Wait);
    assert_eq!(gate.request(now, Show::Raise), None);
    gate.commit("skin://new".to_owned(), now);
    assert_eq!(gate.poll(now), SkinRevealEffect::Skinned(Show::Raise));
    assert_eq!(gate.poll(end), SkinRevealEffect::Wait);

    gate.new_document();
    let end = gate
        .request(end, Show::Raise)
        .expect("reopen during reload");
    assert_eq!(
        gate.poll(end),
        SkinRevealEffect::RevealAndReportTimeout(Show::Raise)
    );
}

#[test]
fn a_commit_observed_after_the_deadline_cannot_win() {
    let now = Instant::now();
    let mut gate = SkinRevealGate::default();
    let end = gate
        .request(now, Show::Raise)
        .expect("reopen arms a deadline");
    gate.allocate(Some("skin://current"), now);
    gate.commit("skin://current".to_owned(), end);
    assert_eq!(
        gate.poll(end),
        SkinRevealEffect::RevealAndReportTimeout(Show::Raise)
    );
}

#[test]
fn timely_readiness_survives_a_delayed_ui_turn() {
    let now = Instant::now();
    let mut gate = SkinRevealGate::default();
    let end = gate
        .request(now, Show::First)
        .expect("first reveal arms the deadline");
    gate.allocate(Some("skin://current"), now);
    gate.commit("skin://current".to_owned(), now);
    assert_eq!(gate.poll(end), SkinRevealEffect::Skinned(Show::First));
}
