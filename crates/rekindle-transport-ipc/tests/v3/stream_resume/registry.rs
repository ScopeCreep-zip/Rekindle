use std::time::{Duration, Instant};
use rekindle_transport_ipc::v3::stream::resume::{ResumeState, ResumeRegistry};

fn make_state_with_window(transfer_id: uuid::Uuid, window: Duration, suspended_ago: Duration) -> ResumeState {
    ResumeState::new(
        transfer_id,
        0,
        0,
        [0xAA; 32],
        [0xBB; 32],
        Instant::now() - suspended_ago,
        window,
    )
}

fn fresh_state(transfer_id: uuid::Uuid) -> ResumeState {
    make_state_with_window(transfer_id, Duration::from_secs(300), Duration::ZERO)
}

fn id(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}

#[test]
fn register_creates_entry() {
    let mut reg = ResumeRegistry::new();
    reg.register(fresh_state(id(1)));
    assert!(reg.lookup(id(1)).is_some());
}

#[test]
fn lookup_unknown_returns_none() {
    let reg = ResumeRegistry::new();
    assert!(reg.lookup(id(999)).is_none());
}

#[test]
fn register_overwrites_existing() {
    let mut reg = ResumeRegistry::new();
    let state1 = ResumeState::new(id(1), 100, 5, [0x11; 32], [0x22; 32], Instant::now(), Duration::from_secs(300));
    let state2 = ResumeState::new(id(1), 200, 10, [0x33; 32], [0x44; 32], Instant::now(), Duration::from_secs(300));
    reg.register(state1);
    reg.register(state2);
    let found = reg.lookup(id(1)).unwrap();
    assert_eq!(found.resume_from_byte(), 200);
    assert_eq!(found.resume_from_chunk(), 10);
}

#[test]
fn remove_deletes_entry() {
    let mut reg = ResumeRegistry::new();
    reg.register(fresh_state(id(1)));
    assert!(reg.remove(id(1)));
    assert!(reg.lookup(id(1)).is_none());
}

#[test]
fn remove_nonexistent_returns_false() {
    let mut reg = ResumeRegistry::new();
    assert!(!reg.remove(id(999)));
}

#[test]
fn gc_expired_removes_old_entries() {
    let mut reg = ResumeRegistry::new();
    // Two expired (suspended 301s ago with 300s window)
    reg.register(make_state_with_window(id(1), Duration::from_secs(300), Duration::from_secs(301)));
    reg.register(make_state_with_window(id(2), Duration::from_secs(300), Duration::from_secs(400)));
    // One still valid
    reg.register(make_state_with_window(id(3), Duration::from_secs(300), Duration::from_secs(1)));

    let removed = reg.gc_expired();
    assert_eq!(removed, 2);
    assert!(reg.lookup(id(1)).is_none());
    assert!(reg.lookup(id(2)).is_none());
    assert!(reg.lookup(id(3)).is_some());
}

#[test]
fn gc_preserves_unexpired() {
    let mut reg = ResumeRegistry::new();
    reg.register(fresh_state(id(1)));
    reg.register(fresh_state(id(2)));
    reg.register(fresh_state(id(3)));

    let removed = reg.gc_expired();
    assert_eq!(removed, 0);
    assert_eq!(reg.count(), 3);
}

#[test]
fn gc_empty_registry_is_noop() {
    let mut reg = ResumeRegistry::new();
    let removed = reg.gc_expired();
    assert_eq!(removed, 0);
}

#[test]
fn registry_count() {
    let mut reg = ResumeRegistry::new();
    reg.register(fresh_state(id(1)));
    reg.register(fresh_state(id(2)));
    reg.register(fresh_state(id(3)));
    reg.register(fresh_state(id(4)));
    reg.register(fresh_state(id(5)));
    assert_eq!(reg.count(), 5);
    reg.remove(id(2));
    reg.remove(id(4));
    assert_eq!(reg.count(), 3);
}

#[test]
fn register_after_remove_reuses_id() {
    let mut reg = ResumeRegistry::new();
    reg.register(ResumeState::new(id(1), 100, 5, [0x11; 32], [0x22; 32], Instant::now(), Duration::from_secs(300)));
    reg.remove(id(1));
    reg.register(ResumeState::new(id(1), 999, 50, [0x33; 32], [0x44; 32], Instant::now(), Duration::from_secs(300)));
    let found = reg.lookup(id(1)).unwrap();
    assert_eq!(found.resume_from_byte(), 999);
}
