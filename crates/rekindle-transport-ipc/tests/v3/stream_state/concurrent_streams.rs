use rekindle_transport_ipc::v3::stream::state::StreamState;
use rekindle_transport_ipc::v3::stream::registry::StreamRegistry;

#[test]
fn open_256_streams_simultaneously() {
    let mut reg = StreamRegistry::new();
    for id in 0..=255u8 {
        assert!(
            reg.open(id).is_ok(),
            "StreamId {id} must be openable"
        );
    }
    assert_eq!(reg.active_count(), 256);
}

#[test]
fn stream_id_reusable_after_closed() {
    let mut reg = StreamRegistry::new();
    reg.open(5).unwrap();
    reg.close(5).unwrap();
    assert!(reg.open(5).is_ok(), "StreamId 5 must be reusable after close");
}

#[test]
fn stream_id_not_reusable_while_open() {
    let mut reg = StreamRegistry::new();
    reg.open(5).unwrap();
    let err = reg.open(5).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("AlreadyOpen") || msg.contains("already"),
        "Must reject duplicate open, got: {msg}"
    );
}

#[test]
fn failure_on_one_stream_does_not_affect_others() {
    let mut reg = StreamRegistry::new();
    reg.open(5).unwrap();
    reg.open(7).unwrap();
    reg.reset(5).unwrap();
    assert_eq!(reg.state(5), Some(StreamState::Closed));
    assert_eq!(reg.state(7), Some(StreamState::Open));
}

#[test]
fn stream_id_exhaustion() {
    let mut reg = StreamRegistry::new();
    for id in 0..=255u8 {
        reg.open(id).unwrap();
    }
    // No more IDs available — opening any ID should fail
    // (all 256 are in use, none closed)
    // We can't open ID 0 again because it's already open
    let err = reg.open(0).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("AlreadyOpen") || msg.contains("Exhausted") || msg.contains("already"),
        "Must reject when all IDs in use, got: {msg}"
    );
}

#[test]
fn active_count_decrements_on_close() {
    let mut reg = StreamRegistry::new();
    reg.open(0).unwrap();
    reg.open(1).unwrap();
    reg.open(2).unwrap();
    assert_eq!(reg.active_count(), 3);
    reg.close(1).unwrap();
    assert_eq!(reg.active_count(), 2);
}

#[test]
fn state_returns_none_for_unknown_id() {
    let reg = StreamRegistry::new();
    assert_eq!(reg.state(99), None);
}

#[test]
fn close_unknown_id_is_error() {
    let mut reg = StreamRegistry::new();
    assert!(reg.close(99).is_err());
}

#[test]
fn reset_transitions_to_closed() {
    let mut reg = StreamRegistry::new();
    reg.open(10).unwrap();
    reg.reset(10).unwrap();
    assert_eq!(reg.state(10), Some(StreamState::Closed));
}
