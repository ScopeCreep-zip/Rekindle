use rekindle_transport_ipc::v3::stream::state::StreamState;
use rekindle_transport_ipc::v3::stream::registry::{Direction, StreamRegistry};

#[test]
fn open_256_streams_simultaneously() {
    let mut reg = StreamRegistry::new();
    for id in 0..=255u8 {
        assert!(
            reg.open(id, Direction::Outbound).is_ok(),
            "StreamId {id} must be openable"
        );
    }
    assert_eq!(reg.active_count(), 256);
}

#[test]
fn stream_id_reusable_after_closed() {
    let mut reg = StreamRegistry::new();
    reg.open(5, Direction::Outbound).unwrap();
    reg.close(5, Direction::Outbound).unwrap();
    assert!(reg.open(5, Direction::Outbound).is_ok(), "StreamId 5 must be reusable after close");
}

#[test]
fn stream_id_not_reusable_while_open_inbound() {
    let mut reg = StreamRegistry::new();
    reg.open(5, Direction::Inbound).unwrap();
    let err = reg.open(5, Direction::Inbound).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("AlreadyOpen") || msg.contains("already"),
        "Must reject duplicate inbound open, got: {msg}"
    );
}

#[test]
fn failure_on_one_stream_does_not_affect_others() {
    let mut reg = StreamRegistry::new();
    reg.open(5, Direction::Outbound).unwrap();
    reg.open(7, Direction::Outbound).unwrap();
    reg.reset(5, Direction::Outbound).unwrap();
    assert_eq!(reg.state(5, Direction::Outbound), Some(StreamState::Closed));
    assert_eq!(reg.state(7, Direction::Outbound), Some(StreamState::Open));
}

#[test]
fn stream_id_exhaustion() {
    let mut reg = StreamRegistry::new();
    for id in 0..=255u8 {
        reg.open(id, Direction::Outbound).unwrap();
    }
    // Same direction, same ID — outbound force-closes stale, so this succeeds.
    // Test inbound exhaustion instead for a real rejection.
    let mut reg2 = StreamRegistry::new();
    for id in 0..=255u8 {
        reg2.open(id, Direction::Inbound).unwrap();
    }
    let err = reg2.open(0, Direction::Inbound).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("AlreadyOpen") || msg.contains("Exhausted") || msg.contains("already"),
        "Must reject when all inbound IDs in use, got: {msg}"
    );
}

#[test]
fn active_count_decrements_on_close() {
    let mut reg = StreamRegistry::new();
    reg.open(0, Direction::Outbound).unwrap();
    reg.open(1, Direction::Outbound).unwrap();
    reg.open(2, Direction::Outbound).unwrap();
    assert_eq!(reg.active_count(), 3);
    reg.close(1, Direction::Outbound).unwrap();
    assert_eq!(reg.active_count(), 2);
}

#[test]
fn state_returns_none_for_unknown_id() {
    let reg = StreamRegistry::new();
    assert_eq!(reg.state(99, Direction::Outbound), None);
}

#[test]
fn close_unknown_id_is_error() {
    let mut reg = StreamRegistry::new();
    assert!(reg.close(99, Direction::Outbound).is_err());
}

#[test]
fn reset_transitions_to_closed() {
    let mut reg = StreamRegistry::new();
    reg.open(10, Direction::Outbound).unwrap();
    reg.reset(10, Direction::Outbound).unwrap();
    assert_eq!(reg.state(10, Direction::Outbound), Some(StreamState::Closed));
}

#[test]
fn same_id_different_directions_independent() {
    let mut reg = StreamRegistry::new();
    reg.open(5, Direction::Outbound).unwrap();
    reg.open(5, Direction::Inbound).unwrap();
    assert_eq!(reg.active_count(), 2);
    reg.close(5, Direction::Outbound).unwrap();
    assert_eq!(reg.state(5, Direction::Outbound), Some(StreamState::Closed));
    assert_eq!(reg.state(5, Direction::Inbound), Some(StreamState::Open));
}
