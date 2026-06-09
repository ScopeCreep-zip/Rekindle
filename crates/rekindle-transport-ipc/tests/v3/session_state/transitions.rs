use rekindle_transport_ipc::v3::session::state::{
    SessionState, SessionEvent,
};

// ── Valid transitions ─────────────────────────────────────────────

#[test]
fn pending_to_handshaking() {
    let mut state = SessionState::Pending;
    assert!(state.apply(SessionEvent::SubstrateConnected).is_ok());
    assert_eq!(state, SessionState::Handshaking);
}

#[test]
fn handshaking_to_established() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::HelloAckValid).is_ok());
    assert_eq!(state, SessionState::Established);
}

#[test]
fn established_to_draining_via_goodbye_sent() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::GoodbyeSent).is_ok());
    assert_eq!(state, SessionState::Draining);
}

#[test]
fn established_to_draining_via_goodbye_received() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::GoodbyeReceived).is_ok());
    assert_eq!(state, SessionState::Draining);
}

#[test]
fn established_to_rotating() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::RotateInitSent).is_ok());
    assert_eq!(state, SessionState::Rotating);
}

#[test]
fn established_to_quiesced() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::QuiesceAcked).is_ok());
    assert_eq!(state, SessionState::Quiesced);
}

#[test]
fn established_to_closed_heartbeat_timeout() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::HeartbeatTimeout).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_aead_failed() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::AeadFailed).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_emac_failed() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::EmacFailed).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_header_mac_failed() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::HeaderMacFailed).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_substrate_eof() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::SubstrateEof).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_substrate_error() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::SubstrateError).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_channel_error() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::ChannelErrorReceived).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_replay_detected() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::ReplayDetected).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_audit_divergence() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::AuditChainDivergence).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_frame_class_unknown() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::FrameClassUnknown).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_lane_unknown() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::LaneUnknown).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn established_to_closed_reserved_bit_set() {
    let mut state = SessionState::Established;
    assert!(state.apply(SessionEvent::ReservedBitSet).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn rotating_to_established_via_commit_valid() {
    let mut state = SessionState::Rotating;
    assert!(state.apply(SessionEvent::RotateCommitValid).is_ok());
    assert_eq!(state, SessionState::Established);
}

#[test]
fn rotating_to_established_via_commit_sent() {
    let mut state = SessionState::Rotating;
    assert!(state.apply(SessionEvent::RotateCommitSent).is_ok());
    assert_eq!(state, SessionState::Established);
}

#[test]
fn rotating_to_closed_timeout() {
    let mut state = SessionState::Rotating;
    assert!(state.apply(SessionEvent::RotationTimeout).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn quiesced_to_established() {
    let mut state = SessionState::Quiesced;
    assert!(state.apply(SessionEvent::ResumeAcked).is_ok());
    assert_eq!(state, SessionState::Established);
}

#[test]
fn quiesced_to_closed_timeout() {
    let mut state = SessionState::Quiesced;
    assert!(state.apply(SessionEvent::QuiescenceTimeout).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn quiesced_to_closed_substrate_disconnect() {
    let mut state = SessionState::Quiesced;
    assert!(state.apply(SessionEvent::SubstrateEof).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn draining_to_closed_complete() {
    let mut state = SessionState::Draining;
    assert!(state.apply(SessionEvent::BothGoodbyeAcked).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn draining_to_closed_timeout() {
    let mut state = SessionState::Draining;
    assert!(state.apply(SessionEvent::DrainTimeout).is_ok());
    assert_eq!(state, SessionState::Closed);
}

// ── Handshake failure transitions ─────────────────────────────────

#[test]
fn handshaking_to_closed_timeout() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::HandshakeTimeout).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn handshaking_to_closed_noise_failed() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::HandshakeNoiseFailed).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn handshaking_to_closed_peer_unregistered() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::PeerUnregistered).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn handshaking_to_closed_capability_mismatch() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::CapabilityMismatch).is_ok());
    assert_eq!(state, SessionState::Closed);
}

#[test]
fn handshaking_to_closed_substrate_error() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::SubstrateError).is_ok());
    assert_eq!(state, SessionState::Closed);
}

// ── Invalid transitions ───────────────────────────────────────────

#[test]
fn closed_rejects_all_events() {
    let events = SessionEvent::all_variants();
    for &event in events {
        let mut state = SessionState::Closed;
        let result = state.apply(event);
        assert!(
            result.is_err(),
            "Closed must reject {event:?} but accepted it"
        );
        assert_eq!(state, SessionState::Closed);
    }
}

#[test]
fn pending_rejects_goodbye() {
    let mut state = SessionState::Pending;
    assert!(state.apply(SessionEvent::GoodbyeSent).is_err());
    assert_eq!(state, SessionState::Pending);
}

#[test]
fn pending_rejects_rotate() {
    let mut state = SessionState::Pending;
    assert!(state.apply(SessionEvent::RotateInitSent).is_err());
}

#[test]
fn handshaking_rejects_rotate() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::RotateInitSent).is_err());
}

#[test]
fn handshaking_rejects_goodbye() {
    let mut state = SessionState::Handshaking;
    assert!(state.apply(SessionEvent::GoodbyeSent).is_err());
}

#[test]
fn rotating_rejects_goodbye() {
    let mut state = SessionState::Rotating;
    assert!(state.apply(SessionEvent::GoodbyeSent).is_err());
}

#[test]
fn rotating_rejects_quiesce() {
    let mut state = SessionState::Rotating;
    assert!(state.apply(SessionEvent::QuiesceAcked).is_err());
}

#[test]
fn quiesced_rejects_rotate() {
    let mut state = SessionState::Quiesced;
    assert!(state.apply(SessionEvent::RotateInitSent).is_err());
}

#[test]
fn quiesced_rejects_goodbye() {
    let mut state = SessionState::Quiesced;
    assert!(state.apply(SessionEvent::GoodbyeSent).is_err());
}

#[test]
fn draining_rejects_rotate() {
    let mut state = SessionState::Draining;
    assert!(state.apply(SessionEvent::RotateInitSent).is_err());
}

#[test]
fn draining_rejects_quiesce() {
    let mut state = SessionState::Draining;
    assert!(state.apply(SessionEvent::QuiesceAcked).is_err());
}

// ── State is not mutated on rejected transition ───────────────────

#[test]
fn rejected_transition_preserves_state() {
    let mut state = SessionState::Pending;
    let _ = state.apply(SessionEvent::GoodbyeSent);
    assert_eq!(state, SessionState::Pending, "state must not change on rejected transition");
}

// ── Error type is specific ────────────────────────────────────────

#[test]
fn rejected_transition_names_from_and_event() {
    let mut state = SessionState::Pending;
    let err = state.apply(SessionEvent::GoodbyeSent).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Pending") && msg.contains("GoodbyeSent"),
        "Error must name both the state and event, got: {msg}"
    );
}
