use crate::broadcast::peer_registry::RelayState;

use super::PROBE_COOLDOWN_SECS;

#[test]
fn only_the_friend_asked_about_that_target_may_answer_once() {
    let relay = RelayState::default();
    relay.record_probe("bob", "carol", 100, PROBE_COOLDOWN_SECS);
    // Another friend, or Bob about someone else: unasked.
    assert!(!relay.take_probe_answer("mallory", "carol", 101, PROBE_COOLDOWN_SECS));
    assert!(!relay.take_probe_answer("bob", "dave", 101, PROBE_COOLDOWN_SECS));
    assert!(relay.take_probe_answer("bob", "carol", 101, PROBE_COOLDOWN_SECS));
    // One probe takes one answer.
    assert!(!relay.take_probe_answer("bob", "carol", 102, PROBE_COOLDOWN_SECS));
}

#[test]
fn a_late_answer_is_unasked() {
    let relay = RelayState::default();
    relay.record_probe("bob", "carol", 100, PROBE_COOLDOWN_SECS);
    assert!(!relay.take_probe_answer(
        "bob",
        "carol",
        100 + PROBE_COOLDOWN_SECS + 1,
        PROBE_COOLDOWN_SECS
    ));
}
