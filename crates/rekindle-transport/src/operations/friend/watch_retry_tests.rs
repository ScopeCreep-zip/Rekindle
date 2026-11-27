use rekindle_utils::OsRandom;

use super::*;

fn profile(peer: &str) -> WatchTarget {
    WatchTarget::Profile {
        peer: peer.to_string(),
    }
}

#[test]
fn the_backoff_doubles_with_jitter_up_to_the_cap() {
    let rng = OsRandom;
    for attempts in 1..40 {
        let step = (FIRST_DELAY_MS << (attempts - 1).min(16)).min(MAX_DELAY_MS);
        let wait = backoff(attempts, &rng);
        assert!(
            wait >= step / 2 && wait <= step,
            "attempt {attempts}: {wait}"
        );
    }
}

#[test]
fn a_failure_is_due_after_its_backoff_until_it_settles() {
    let rng = OsRandom;
    let mut retries = WatchRetries::default();
    retries.failed("rec", profile("ann"), 1_000, &rng);
    assert!(retries.due(1_000 + FIRST_DELAY_MS / 2 - 1).is_empty());
    assert_eq!(
        retries.due(1_000 + FIRST_DELAY_MS),
        vec![("rec".to_string(), profile("ann"))]
    );
    retries.failed("rec", profile("ann"), 50_000, &rng);
    assert!(retries.due(50_000 + FIRST_DELAY_MS - 1).is_empty());
    assert_eq!(retries.due(50_000 + 2 * FIRST_DELAY_MS).len(), 1);
    retries.settled("rec");
    assert!(!retries.is_pending("rec"));
}

#[test]
fn an_unwatch_and_a_lock_forget_the_failures() {
    let rng = OsRandom;
    let mut retries = WatchRetries::default();
    retries.failed("p", profile("ann"), 0, &rng);
    let dm = WatchTarget::Participant {
        key: "ann".to_string(),
    };
    retries.failed("d", dm, 0, &rng);
    let group = WatchTarget::Participant {
        key: "g:bea".to_string(),
    };
    retries.failed("g", group, 0, &rng);
    retries.forget("ann");
    assert!(!retries.is_pending("p") && !retries.is_pending("d"));
    assert!(retries.is_pending("g"));
    retries.clear();
    assert!(!retries.is_pending("g"));
}
