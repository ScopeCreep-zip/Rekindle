use rekindle_transport_ipc::v3::stream::state::{
    StreamState, StreamEvent,
};

// ── Valid transitions ─────────────────────────────────────────────

#[test]
fn idle_to_opening() {
    let mut state = StreamState::Idle;
    assert!(state.apply(StreamEvent::OpenSent).is_ok());
    assert_eq!(state, StreamState::Opening);
}

#[test]
fn opening_to_open_via_first_ack() {
    let mut state = StreamState::Opening;
    assert!(state.apply(StreamEvent::FirstAckReceived).is_ok());
    assert_eq!(state, StreamState::Open);
}

#[test]
fn opening_to_open_via_implicit_window() {
    let mut state = StreamState::Opening;
    assert!(state.apply(StreamEvent::ImplicitAckWindowExpired).is_ok());
    assert_eq!(state, StreamState::Open);
}

#[test]
fn opening_to_failed_via_nack() {
    let mut state = StreamState::Opening;
    assert!(state.apply(StreamEvent::NackReceived).is_ok());
    assert_eq!(state, StreamState::Failed);
}

#[test]
fn open_to_closing() {
    let mut state = StreamState::Open;
    assert!(state.apply(StreamEvent::FinSent).is_ok());
    assert_eq!(state, StreamState::Closing);
}

#[test]
fn closing_to_closed() {
    let mut state = StreamState::Closing;
    assert!(state.apply(StreamEvent::AckForFinReceived).is_ok());
    assert_eq!(state, StreamState::Closed);
}

#[test]
fn open_to_resetting() {
    let mut state = StreamState::Open;
    assert!(state.apply(StreamEvent::ResetSent).is_ok());
    assert_eq!(state, StreamState::Resetting);
}

#[test]
fn open_to_resetting_via_received() {
    let mut state = StreamState::Open;
    assert!(state.apply(StreamEvent::ResetReceived).is_ok());
    assert_eq!(state, StreamState::Resetting);
}

#[test]
fn resetting_to_closed() {
    let mut state = StreamState::Resetting;
    assert!(state.apply(StreamEvent::CleanupComplete).is_ok());
    assert_eq!(state, StreamState::Closed);
}

#[test]
fn open_to_suspended() {
    let mut state = StreamState::Open;
    assert!(state.apply(StreamEvent::SubstrateFailure).is_ok());
    assert_eq!(state, StreamState::Suspended);
}

#[test]
fn suspended_to_open_via_resume() {
    let mut state = StreamState::Suspended;
    assert!(state.apply(StreamEvent::ResumeAccepted).is_ok());
    assert_eq!(state, StreamState::Open);
}

#[test]
fn suspended_to_failed_via_resume_denied() {
    let mut state = StreamState::Suspended;
    assert!(state.apply(StreamEvent::ResumeDenied).is_ok());
    assert_eq!(state, StreamState::Failed);
}

#[test]
fn suspended_to_failed_via_window_expired() {
    let mut state = StreamState::Suspended;
    assert!(state.apply(StreamEvent::WindowExpired).is_ok());
    assert_eq!(state, StreamState::Failed);
}

#[test]
fn open_to_closing_via_cancel_sent() {
    let mut state = StreamState::Open;
    assert!(state.apply(StreamEvent::CancelSent).is_ok());
    assert_eq!(state, StreamState::Closing);
}

#[test]
fn closing_to_closed_via_cancel_ack() {
    let mut state = StreamState::Closing;
    assert!(state.apply(StreamEvent::CancelAckReceived).is_ok());
    assert_eq!(state, StreamState::Closed);
}

// ── Invalid transitions ───────────────────────────────────────────

#[test]
fn closed_rejects_all_events() {
    let events = StreamEvent::all_variants();
    for &event in events {
        let mut state = StreamState::Closed;
        assert!(
            state.apply(event).is_err(),
            "Closed must reject {event:?}"
        );
    }
}

#[test]
fn failed_rejects_all_events() {
    let events = StreamEvent::all_variants();
    for &event in events {
        let mut state = StreamState::Failed;
        assert!(
            state.apply(event).is_err(),
            "Failed must reject {event:?}"
        );
    }
}

#[test]
fn idle_rejects_payload() {
    let mut state = StreamState::Idle;
    assert!(state.apply(StreamEvent::PayloadReceived).is_err());
}

#[test]
fn idle_rejects_fin() {
    let mut state = StreamState::Idle;
    assert!(state.apply(StreamEvent::FinSent).is_err());
}

#[test]
fn closing_rejects_new_payload() {
    let mut state = StreamState::Closing;
    assert!(state.apply(StreamEvent::PayloadReceived).is_err());
}

#[test]
fn opening_rejects_fin() {
    let mut state = StreamState::Opening;
    assert!(state.apply(StreamEvent::FinSent).is_err());
}

#[test]
fn suspended_rejects_payload() {
    let mut state = StreamState::Suspended;
    assert!(state.apply(StreamEvent::PayloadReceived).is_err());
}

// ── State not mutated on rejection ────────────────────────────────

#[test]
fn rejected_transition_preserves_state() {
    let mut state = StreamState::Idle;
    let _ = state.apply(StreamEvent::FinSent);
    assert_eq!(state, StreamState::Idle);
}

// ── Error is specific ─────────────────────────────────────────────

#[test]
fn error_names_state_and_event() {
    let mut state = StreamState::Idle;
    let err = state.apply(StreamEvent::FinSent).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Idle") && msg.contains("FinSent"),
        "Error must name state and event, got: {msg}"
    );
}
