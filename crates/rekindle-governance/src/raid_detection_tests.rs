use super::*;

fn policy(max: u32, interval: u32) -> CommunityPolicyState {
    CommunityPolicyState {
        policy_text: None,
        max_joins_per_interval: max,
        join_interval_seconds: interval,
        stamp: crate::fixture::stamp(1),
    }
}

#[test]
fn observe_below_threshold_returns_none() {
    let mut window = VecDeque::new();
    let p = policy(5, 60);
    for i in 0..4 {
        assert!(observe_join(&mut window, 100 + i, &format!("p{i}"), Some(&p)).is_none());
    }
    assert_eq!(window.len(), 4);
}

#[test]
fn observe_at_threshold_returns_alert() {
    let mut window = VecDeque::new();
    let p = policy(3, 60);
    observe_join(&mut window, 100, "p1", Some(&p));
    observe_join(&mut window, 101, "p2", Some(&p));
    let alert = observe_join(&mut window, 102, "p3", Some(&p));
    assert_eq!(
        alert,
        Some(RaidAlert {
            joins_in_window: 3,
            max_joins_per_interval: 3,
            join_interval_seconds: 60,
        })
    );
}

#[test]
fn old_entries_are_evicted_outside_window() {
    let mut window = VecDeque::new();
    let p = policy(3, 60);
    observe_join(&mut window, 100, "p1", Some(&p));
    observe_join(&mut window, 200, "p2", Some(&p)); // p1 is now stale
    let alert = observe_join(&mut window, 200, "p3", Some(&p));
    assert!(alert.is_none(), "only p2+p3 are inside the 60s window");
    assert_eq!(window.len(), 2);
}

#[test]
fn duplicate_pseudonym_in_window_is_not_double_counted() {
    let mut window = VecDeque::new();
    let p = policy(3, 60);
    observe_join(&mut window, 100, "alice", Some(&p));
    observe_join(&mut window, 101, "alice", Some(&p));
    observe_join(&mut window, 102, "alice", Some(&p));
    assert_eq!(window.len(), 1);
}

#[test]
fn missing_policy_uses_architecture_defaults() {
    let (max, interval) = resolve_thresholds(None);
    assert_eq!(max, 20);
    assert_eq!(interval, 600);
}
