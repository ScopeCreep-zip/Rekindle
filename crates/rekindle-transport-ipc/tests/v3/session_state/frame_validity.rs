use rekindle_transport_ipc::v3::session::state::{SessionState, is_frame_allowed};
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::{ChannelKind, StreamKind, DatagramKind, AuditKind, HandoffKind};

// ── Pending allows nothing ────────────────────────────────────────

#[test]
fn pending_rejects_all_classes() {
    for &class in &FrameClass::ALL {
        assert!(
            !is_frame_allowed(SessionState::Pending, class, 0x01),
            "Pending must reject {class:?}"
        );
    }
}

// ── Handshaking allows only Hello / HelloAck ──────────────────────

#[test]
fn handshaking_allows_hello() {
    assert!(is_frame_allowed(
        SessionState::Handshaking,
        FrameClass::Channel,
        ChannelKind::Hello as u8,
    ));
}

#[test]
fn handshaking_allows_hello_ack() {
    assert!(is_frame_allowed(
        SessionState::Handshaking,
        FrameClass::Channel,
        ChannelKind::HelloAck as u8,
    ));
}

#[test]
fn handshaking_rejects_ping() {
    assert!(!is_frame_allowed(
        SessionState::Handshaking,
        FrameClass::Channel,
        ChannelKind::Ping as u8,
    ));
}

#[test]
fn handshaking_rejects_stream() {
    assert!(!is_frame_allowed(
        SessionState::Handshaking,
        FrameClass::Stream,
        StreamKind::Payload as u8,
    ));
}

#[test]
fn handshaking_rejects_datagram() {
    assert!(!is_frame_allowed(
        SessionState::Handshaking,
        FrameClass::Datagram,
        DatagramKind::Request as u8,
    ));
}

#[test]
fn handshaking_rejects_audit() {
    assert!(!is_frame_allowed(
        SessionState::Handshaking,
        FrameClass::Audit,
        AuditKind::Checkpoint as u8,
    ));
}

#[test]
fn handshaking_rejects_handoff() {
    assert!(!is_frame_allowed(
        SessionState::Handshaking,
        FrameClass::Handoff,
        HandoffKind::Offer as u8,
    ));
}

// ── Established allows everything ─────────────────────────────────

#[test]
fn established_allows_all_channel_kinds() {
    for &kind in ChannelKind::all_variants() {
        assert!(
            is_frame_allowed(SessionState::Established, FrameClass::Channel, kind as u8),
            "Established must allow ChannelKind::{kind:?}"
        );
    }
}

#[test]
fn established_allows_all_stream_kinds() {
    for &kind in StreamKind::all_variants() {
        assert!(
            is_frame_allowed(SessionState::Established, FrameClass::Stream, kind as u8),
            "Established must allow StreamKind::{kind:?}"
        );
    }
}

#[test]
fn established_allows_all_datagram_kinds() {
    for &kind in DatagramKind::all_variants() {
        assert!(
            is_frame_allowed(SessionState::Established, FrameClass::Datagram, kind as u8),
            "Established must allow DatagramKind::{kind:?}"
        );
    }
}

#[test]
fn established_allows_all_audit_kinds() {
    for &kind in AuditKind::all_variants() {
        assert!(
            is_frame_allowed(SessionState::Established, FrameClass::Audit, kind as u8),
            "Established must allow AuditKind::{kind:?}"
        );
    }
}

#[test]
fn established_allows_all_handoff_kinds() {
    for &kind in HandoffKind::all_variants() {
        assert!(
            is_frame_allowed(SessionState::Established, FrameClass::Handoff, kind as u8),
            "Established must allow HandoffKind::{kind:?}"
        );
    }
}

// ── Rotating allows in-flight frames, rejects lifecycle conflicts ──
//
// Rotating is transient. In-flight frames from before rotation started
// (replies, bulk chunks, audit, handoff) must be accepted. Only lifecycle
// changes that conflict with rotation are rejected.

#[test]
fn rotating_allows_rotate_init() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Channel, ChannelKind::RotateInit as u8,
    ));
}

#[test]
fn rotating_allows_rotate_commit() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Channel, ChannelKind::RotateCommit as u8,
    ));
}

#[test]
fn rotating_allows_ping() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Channel, ChannelKind::Ping as u8,
    ));
}

#[test]
fn rotating_allows_pong() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Channel, ChannelKind::Pong as u8,
    ));
}

#[test]
fn rotating_allows_ack() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Channel, ChannelKind::Ack as u8,
    ));
}

#[test]
fn rotating_allows_datagram_reply() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Datagram, DatagramKind::Reply as u8,
    ));
}

#[test]
fn rotating_allows_datagram_request() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Datagram, DatagramKind::Request as u8,
    ));
}

#[test]
fn rotating_allows_stream_payload() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Stream, StreamKind::Payload as u8,
    ));
}

#[test]
fn rotating_rejects_stream_open() {
    assert!(!is_frame_allowed(
        SessionState::Rotating, FrameClass::Stream, StreamKind::Open as u8,
    ));
}

#[test]
fn rotating_allows_audit_checkpoint() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Audit, AuditKind::Checkpoint as u8,
    ));
}

#[test]
fn rotating_allows_handoff() {
    assert!(is_frame_allowed(
        SessionState::Rotating, FrameClass::Handoff, HandoffKind::Offer as u8,
    ));
}

#[test]
fn rotating_rejects_goodbye() {
    assert!(!is_frame_allowed(
        SessionState::Rotating, FrameClass::Channel, ChannelKind::Goodbye as u8,
    ));
}

#[test]
fn rotating_rejects_quiesce() {
    assert!(!is_frame_allowed(
        SessionState::Rotating, FrameClass::Channel, ChannelKind::Quiesce as u8,
    ));
}

// ── Quiesced allows only Ping / Pong / Resume / ResumeAck ─────────

#[test]
fn quiesced_allows_ping() {
    assert!(is_frame_allowed(
        SessionState::Quiesced,
        FrameClass::Channel,
        ChannelKind::Ping as u8,
    ));
}

#[test]
fn quiesced_allows_pong() {
    assert!(is_frame_allowed(
        SessionState::Quiesced,
        FrameClass::Channel,
        ChannelKind::Pong as u8,
    ));
}

#[test]
fn quiesced_allows_resume() {
    assert!(is_frame_allowed(
        SessionState::Quiesced,
        FrameClass::Channel,
        ChannelKind::Resume as u8,
    ));
}

#[test]
fn quiesced_allows_resume_ack() {
    assert!(is_frame_allowed(
        SessionState::Quiesced,
        FrameClass::Channel,
        ChannelKind::ResumeAck as u8,
    ));
}

#[test]
fn quiesced_rejects_stream() {
    assert!(!is_frame_allowed(
        SessionState::Quiesced,
        FrameClass::Stream,
        StreamKind::Open as u8,
    ));
}

#[test]
fn quiesced_rejects_datagram() {
    assert!(!is_frame_allowed(
        SessionState::Quiesced,
        FrameClass::Datagram,
        DatagramKind::Publish as u8,
    ));
}

#[test]
fn quiesced_rejects_goodbye() {
    assert!(!is_frame_allowed(
        SessionState::Quiesced,
        FrameClass::Channel,
        ChannelKind::Goodbye as u8,
    ));
}

// ── Draining allows Ping/Pong/Ack/GoodbyeAck, rejects S/D/H ─────

#[test]
fn draining_allows_ping() {
    assert!(is_frame_allowed(
        SessionState::Draining,
        FrameClass::Channel,
        ChannelKind::Ping as u8,
    ));
}

#[test]
fn draining_allows_pong() {
    assert!(is_frame_allowed(
        SessionState::Draining,
        FrameClass::Channel,
        ChannelKind::Pong as u8,
    ));
}

#[test]
fn draining_allows_ack() {
    assert!(is_frame_allowed(
        SessionState::Draining,
        FrameClass::Channel,
        ChannelKind::Ack as u8,
    ));
}

#[test]
fn draining_allows_goodbye_ack() {
    assert!(is_frame_allowed(
        SessionState::Draining,
        FrameClass::Channel,
        ChannelKind::GoodbyeAck as u8,
    ));
}

#[test]
fn draining_rejects_new_stream() {
    assert!(!is_frame_allowed(
        SessionState::Draining,
        FrameClass::Stream,
        StreamKind::Open as u8,
    ));
}

#[test]
fn draining_rejects_datagram() {
    assert!(!is_frame_allowed(
        SessionState::Draining,
        FrameClass::Datagram,
        DatagramKind::Request as u8,
    ));
}

#[test]
fn draining_rejects_handoff() {
    assert!(!is_frame_allowed(
        SessionState::Draining,
        FrameClass::Handoff,
        HandoffKind::Offer as u8,
    ));
}

#[test]
fn draining_allows_audit_checkpoint() {
    assert!(is_frame_allowed(
        SessionState::Draining,
        FrameClass::Audit,
        AuditKind::Checkpoint as u8,
    ));
}

#[test]
fn draining_rejects_audit_gap() {
    assert!(!is_frame_allowed(
        SessionState::Draining,
        FrameClass::Audit,
        AuditKind::Gap as u8,
    ));
}

// ── Closed allows nothing ─────────────────────────────────────────

#[test]
fn closed_rejects_all_classes() {
    for &class in &FrameClass::ALL {
        assert!(
            !is_frame_allowed(SessionState::Closed, class, 0x01),
            "Closed must reject {class:?}"
        );
    }
}
