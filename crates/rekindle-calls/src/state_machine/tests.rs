//! Tests for [`super::CallStateMachine`].

use super::*;
use crate::fresh_keypair;

fn fresh_event_local_start(call_id: &str, peer: &str, kind: CallKind) -> CallInput {
    let (sk, pk) = fresh_keypair();
    CallInput::LocalStartCall {
        call_id: call_id.into(),
        peer: peer.into(),
        peer_display_name: format!("name-of-{peer}"),
        kind,
        my_x25519_secret: sk,
        my_x25519_pub: pk,
        expires_at_ms: 30_000,
        started_at_ms: 0,
    }
}

fn fresh_event_invite_received(call_id: &str, from: &str, kind: CallKind) -> CallInput {
    let (_, peer_pub) = fresh_keypair();
    CallInput::InviteReceived {
        call_id: call_id.into(),
        from: from.into(),
        from_display_name: format!("name-of-{from}"),
        kind,
        peer_x25519_pub: peer_pub,
        expires_at_ms: 30_000,
        received_at_ms: 0,
    }
}

#[test]
fn local_start_emits_invite_timer_persist_notify() {
    let mut sm = CallStateMachine::new();
    let effects = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SendCallInvite { .. })));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SpawnDialingTimer { .. })));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::PersistCallState {
            status: CallStatus::Outgoing,
            ..
        }
    )));
    assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Outgoing)));
}

#[test]
fn invite_received_emits_timer_persist_notify_no_outbound_ringing() {
    // W16.5b — receiver no longer emits an outbound CallRinging
    // envelope; the runtime synthesizes the ringing reply
    // synchronously inside `app_call_reply`.
    let mut sm = CallStateMachine::new();
    let effects = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Video));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SpawnIncomingTimer { .. })));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::PersistCallState {
            status: CallStatus::Incoming,
            ..
        }
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Notify(TransportNotification::IncomingCall { .. })
    )));
    assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Incoming)));
}

#[test]
fn local_unreachable_drops_state_emits_notification() {
    // W16.5b — caller's app_call CallInvite failed inside Veilid's
    // RPC budget. Drops Outgoing, emits CallUnreachable, no
    // missed_call row (receiver never saw the invite).
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let effects = sm.apply(CallInput::LocalUnreachable {
        call_id: "c1".into(),
        reason: "timeout".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::CancelTimer { .. })));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::DeletePersistedCall { .. })));
    assert!(effects.iter().any(|e| matches!(e,
        Effect::Notify(TransportNotification::CallUnreachable { reason, .. }) if reason == "timeout"
    )));
    // No PersistMissedCall — receiver never knew about the call.
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::PersistMissedCall { .. })));
    assert!(sm.get("c1").is_none());
}

#[test]
fn local_unreachable_after_accept_is_noop() {
    // Race: accept arrived before app_call returned its error path.
    // Treat the call as alive and ignore the unreachable signal.
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let (_, peer_pub) = fresh_keypair();
    let _ = sm.apply(CallInput::AcceptReceived {
        call_id: "c1".into(),
        from: "bob".into(),
        peer_x25519_pub: peer_pub,
    });
    let effects = sm.apply(CallInput::LocalUnreachable {
        call_id: "c1".into(),
        reason: "timeout".into(),
    });
    assert!(effects.is_empty(), "unreachable post-accept must be noop");
    assert!(sm.get("c1").is_some(), "call still alive after accept");
}

#[test]
fn duplicate_invite_for_same_call_id_dropped() {
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
    let effects = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
    assert!(effects.is_empty(), "duplicate invite produces no effects");
}

#[test]
fn outgoing_then_decline_received_transitions_to_dropped() {
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let effects = sm.apply(CallInput::DeclineReceived {
        call_id: "c1".into(),
        reason: "busy".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::CancelTimer { .. })));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::DeletePersistedCall { .. })));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Notify(TransportNotification::CallDeclined { .. })
    )));
    assert!(sm.get("c1").is_none(), "call removed after decline");
}

#[test]
fn dialing_timeout_persists_missed_call() {
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let effects = sm.apply(CallInput::LocalDialingTimeout {
        call_id: "c1".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PersistMissedCall { .. })));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Notify(TransportNotification::CallTimedOut { .. })
    )));
    assert!(sm.get("c1").is_none());
}

#[test]
fn incoming_timeout_after_accept_does_nothing() {
    // Accept transitioned to Connecting → timer firing later is a
    // no-op (still_incoming check).
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
    let (sk, pk) = fresh_keypair();
    let _ = sm.apply(CallInput::LocalAccept {
        call_id: "c1".into(),
        my_x25519_secret: sk,
        my_x25519_pub: pk,
    });
    let effects = sm.apply(CallInput::LocalIncomingTimeout {
        call_id: "c1".into(),
    });
    assert!(effects.is_empty(), "timeout after accept is a no-op");
}

#[test]
fn end_received_drops_call_in_any_state() {
    // Outgoing case
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let effects = sm.apply(CallInput::EndReceived {
        call_id: "c1".into(),
        reason: "cancelled".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::Notify(TransportNotification::CallEnded { .. }))));
    assert!(sm.get("c1").is_none());

    // Incoming case
    let mut sm2 = CallStateMachine::new();
    let _ = sm2.apply(fresh_event_invite_received("c2", "alice", CallKind::Audio));
    let effects = sm2.apply(CallInput::EndReceived {
        call_id: "c2".into(),
        reason: "caller hung up".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::Notify(TransportNotification::CallEnded { .. }))));
    assert!(sm2.get("c2").is_none());
}

#[test]
fn voice_transport_up_transitions_connecting_to_active() {
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
    let (sk, pk) = fresh_keypair();
    let _ = sm.apply(CallInput::LocalAccept {
        call_id: "c1".into(),
        my_x25519_secret: sk,
        my_x25519_pub: pk,
    });
    let effects = sm.apply(CallInput::VoiceTransportUp {
        call_id: "c1".into(),
    });
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Notify(TransportNotification::CallConnected { .. })
    )));
    assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Active)));
}

#[test]
fn voice_transport_down_active_cleans_up_and_sends_end() {
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
    let (sk, pk) = fresh_keypair();
    let _ = sm.apply(CallInput::LocalAccept {
        call_id: "c1".into(),
        my_x25519_secret: sk,
        my_x25519_pub: pk,
    });
    let _ = sm.apply(CallInput::VoiceTransportUp {
        call_id: "c1".into(),
    });
    let effects = sm.apply(CallInput::VoiceTransportDown {
        call_id: "c1".into(),
        reason: "network drop".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::StopVoiceSession { .. })));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SendCallEnd { .. })));
    assert!(sm.get("c1").is_none());
}

#[test]
fn ringing_received_only_during_outgoing() {
    let mut sm = CallStateMachine::new();
    // No outgoing — ringing is a no-op.
    let effects = sm.apply(CallInput::RingingReceived {
        call_id: "c1".into(),
    });
    assert!(effects.is_empty());

    // Outgoing — ringing emits notification.
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let effects = sm.apply(CallInput::RingingReceived {
        call_id: "c1".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::Notify(TransportNotification::CallRinging { .. }))));
}

#[test]
fn has_outgoing_to_returns_true_only_for_outgoing() {
    let mut sm = CallStateMachine::new();
    assert!(!sm.has_outgoing_to("bob"));
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    assert!(sm.has_outgoing_to("bob"));
    assert!(!sm.has_outgoing_to("carol"));
}

#[test]
fn accept_received_for_unknown_call_is_noop() {
    let mut sm = CallStateMachine::new();
    let (_, peer_pub) = fresh_keypair();
    let effects = sm.apply(CallInput::AcceptReceived {
        call_id: "unknown".into(),
        from: "bob".into(),
        peer_x25519_pub: peer_pub,
    });
    assert!(effects.is_empty());
}

#[test]
fn accept_received_validates_sender_matches_peer() {
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let (_, attacker_pub) = fresh_keypair();
    let effects = sm.apply(CallInput::AcceptReceived {
        call_id: "c1".into(),
        from: "carol".into(), // not bob — attacker
        peer_x25519_pub: attacker_pub,
    });
    assert!(effects.is_empty(), "wrong sender rejected silently");
}

#[test]
fn dialing_timeout_after_accept_is_noop() {
    // Accept transitioned the state to Connecting — timer that
    // fires later should not produce a missed-call.
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    let (_, bob_pub) = fresh_keypair();
    let _ = sm.apply(CallInput::AcceptReceived {
        call_id: "c1".into(),
        from: "bob".into(),
        peer_x25519_pub: bob_pub,
    });
    let effects = sm.apply(CallInput::LocalDialingTimeout {
        call_id: "c1".into(),
    });
    assert!(effects.is_empty());
}

#[test]
fn full_call_lifecycle_caller_side() {
    let mut sm = CallStateMachine::new();
    let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
    assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Outgoing)));

    let _ = sm.apply(CallInput::RingingReceived {
        call_id: "c1".into(),
    });
    assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Outgoing)));

    let (_, bob_pub) = fresh_keypair();
    let _ = sm.apply(CallInput::AcceptReceived {
        call_id: "c1".into(),
        from: "bob".into(),
        peer_x25519_pub: bob_pub,
    });
    assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Connecting)));

    let _ = sm.apply(CallInput::VoiceTransportUp {
        call_id: "c1".into(),
    });
    assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Active)));

    let _ = sm.apply(CallInput::EndReceived {
        call_id: "c1".into(),
        reason: "peer hung up".into(),
    });
    assert!(sm.get("c1").is_none());
}
