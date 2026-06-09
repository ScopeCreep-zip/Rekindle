use std::time::{Duration, Instant};
use rekindle_transport_ipc::v3::stream::resume::{ResumeState, ResumeConfig};

fn make_state(suspended_ago: Duration, window: Duration) -> ResumeState {
    ResumeState::new(
        uuid::Uuid::nil(),
        1_048_576,
        64,
        [0xAA; 32],
        [0xBB; 32],
        Instant::now() - suspended_ago,
        window,
    )
}

#[test]
fn new_resume_state_captures_all_fields() {
    let transfer_id = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let state = ResumeState::new(
        transfer_id,
        500_000,
        30,
        [0x11; 32],
        [0x22; 32],
        Instant::now(),
        Duration::from_secs(300),
    );
    assert_eq!(state.transfer_id(), transfer_id);
    assert_eq!(state.resume_from_byte(), 500_000);
    assert_eq!(state.resume_from_chunk(), 30);
    assert_eq!(state.anchor_audit_link(), &[0x11; 32]);
    assert_eq!(state.anchor_content_hash(), &[0x22; 32]);
}

#[test]
fn default_config_is_five_minutes() {
    let config = ResumeConfig::default();
    assert_eq!(config.eligibility_window, Duration::from_secs(300));
}

#[test]
fn config_with_window_overrides_default() {
    let config = ResumeConfig::with_window(Duration::from_secs(86400));
    assert_eq!(config.eligibility_window, Duration::from_secs(86400));
}

#[test]
fn state_uses_config_window() {
    let config = ResumeConfig::with_window(Duration::from_secs(600));
    let state = make_state(Duration::from_secs(599), config.eligibility_window);
    assert!(!state.is_expired());

    let state2 = make_state(Duration::from_secs(601), config.eligibility_window);
    assert!(state2.is_expired());
}

#[test]
fn resume_state_not_expired_within_window() {
    let state = make_state(Duration::from_secs(1), Duration::from_secs(300));
    assert!(!state.is_expired());
}

#[test]
fn resume_state_expired_beyond_window() {
    let state = make_state(Duration::from_secs(301), Duration::from_secs(300));
    assert!(state.is_expired());
}

#[test]
fn resume_state_expired_at_exact_boundary() {
    let state = make_state(Duration::from_secs(300), Duration::from_secs(300));
    assert!(state.is_expired());
}

#[test]
fn resume_state_zero_window_always_expired() {
    let state = make_state(Duration::from_millis(1), Duration::ZERO);
    assert!(state.is_expired());
}

#[test]
fn resume_state_large_window_survives() {
    let state = make_state(
        Duration::from_secs(23 * 3600),
        Duration::from_secs(24 * 3600),
    );
    assert!(!state.is_expired());
}
