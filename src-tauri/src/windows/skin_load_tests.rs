use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::ipc::Channel;

use super::{Show, SkinRevealEffect, State, LOAD_LIMIT};

const HREF: &str = "skin://same-skin";

fn begin(state: &mut State, now: Instant) -> String {
    state
        .begin_apply(1, "same", HREF, Channel::new(|_| Ok(())), now)
        .unwrap()
}

#[test]
fn delivered_switch_expires_even_without_a_webview_start() {
    let mut state = State::default();
    let now = Instant::now();
    let (end, _) = state.deliver("same".into(), HREF.into(), now);

    assert_eq!(end, now + LOAD_LIMIT);
    let before = end.checked_sub(Duration::from_millis(1)).unwrap();
    assert!(state.expire_apply(before).is_none());
    let expired = state.expire_apply(end).unwrap();
    assert_eq!(expired.skin, "same");
    assert!(expired.started.is_none());
    assert!(state.expire_apply(end).is_none());

    let notified = Arc::new(AtomicBool::new(false));
    let sent = Arc::clone(&notified);
    let late = Channel::new(move |_| {
        sent.store(true, Ordering::Relaxed);
        Ok(())
    });
    assert!(state.begin_apply(1, "same", HREF, late, end).is_none());
    assert!(notified.load(Ordering::Relaxed));
}

#[test]
fn a_webview_start_does_not_extend_the_delivered_deadline() {
    let mut state = State::default();
    let now = Instant::now();
    let (end, _) = state.deliver("same".into(), HREF.into(), now);
    let before = end.checked_sub(Duration::from_millis(1)).unwrap();
    let started = begin(&mut state, before);

    assert!(!state.finish_apply(&started, Some(HREF.into()), end));
    assert!(state.expire_apply(end).unwrap().started.is_some());
}

#[test]
fn a_live_duplicate_of_a_snapshot_needs_no_new_watchdog() {
    let mut state = State::default();
    let now = Instant::now();
    assert!(state.expects_delivery("same", HREF));
    state.deliver("same".into(), HREF.into(), now);
    assert!(!state.expects_delivery("same", HREF));
    let started = begin(&mut state, now);
    assert!(state.finish_apply(&started, Some(HREF.into()), now));
    assert!(!state.expects_delivery("same", HREF));
    assert!(state.expects_delivery("same", "skin://new-content"));
}

#[test]
fn a_superseded_switch_cannot_claim_or_commit_the_new_delivery() {
    let mut state = State::default();
    let now = Instant::now();
    let (_, superseded) = state.deliver("same".into(), HREF.into(), now);
    let old = begin(&mut state, now);
    let next_href = "skin://new-skin";
    let (_, active) = state.deliver("other".into(), next_href.into(), now);

    assert!(superseded.is_cancelled());
    assert!(state
        .begin_apply(2, "same", HREF, Channel::new(|_| Ok(())), now)
        .is_none());
    assert!(!state.finish_apply(&old, Some(HREF.into()), now));
    assert!(!active.is_cancelled());

    let new = state
        .begin_apply(3, "other", next_href, Channel::new(|_| Ok(())), now)
        .unwrap();
    assert!(state.finish_apply(&new, Some(next_href.into()), now));
    assert!(active.is_cancelled());
}

#[test]
fn a_finish_from_before_reload_cannot_commit_the_new_document() {
    let mut state = State::default();
    let now = Instant::now();
    assert!(state.gate.request(now, Show::First).is_some());
    state.gate.allocate(Some(HREF), now);
    state.deliver("same".into(), HREF.into(), now);
    let old = begin(&mut state, now);
    state.new_document();
    let (_, watchdog) = state.deliver("same".into(), HREF.into(), now);
    let active = begin(&mut state, now);

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
    state.deliver("same".into(), HREF.into(), now);
    let old = begin(&mut state, now);
    state.new_document();
    let (_, watchdog) = state.deliver("same".into(), HREF.into(), now);
    let active = begin(&mut state, now);

    assert!(!state.finish_apply(&old, None, now));
    assert!(!watchdog.is_cancelled());

    assert!(!state.finish_apply(&active, None, now));
    assert!(watchdog.is_cancelled());
    assert!(state.apply.is_none());
}
