use super::{Show, SkinRevealEffect, State};
use std::time::Instant;

const HREF: &str = "skin://same-skin";

#[test]
fn a_finish_from_before_reload_cannot_commit_the_new_document() {
    let mut state = State::default();
    let now = Instant::now();
    assert!(state.gate.request(now, Show::First).is_some());
    state.gate.allocate(Some(HREF), now);
    let (old, _) = state.begin_apply(1, "same".into()).unwrap();
    state.new_document();
    let (active, watchdog) = state.begin_apply(1, "same".into()).unwrap();

    assert!(!state.finish_apply(&old, Some(HREF.into()), now));
    assert_eq!(state.gate.poll(now), SkinRevealEffect::Wait);
    assert!(!watchdog.is_cancelled());

    assert!(state.finish_apply(&active, Some(HREF.into()), now));
    assert!(watchdog.is_cancelled());
    assert_eq!(state.gate.poll(now), SkinRevealEffect::Skinned(Show::First));
}

#[test]
fn an_uncommitted_finish_from_before_reload_keeps_the_new_watchdog() {
    let mut state = State::default();
    let now = Instant::now();
    let (old, _) = state.begin_apply(1, "same".into()).unwrap();
    state.new_document();
    let (active, watchdog) = state.begin_apply(1, "same".into()).unwrap();

    assert!(!state.finish_apply(&old, None, now));
    assert!(!watchdog.is_cancelled());

    assert!(!state.finish_apply(&active, None, now));
    assert!(watchdog.is_cancelled());
    assert!(state.apply.is_none());
}
