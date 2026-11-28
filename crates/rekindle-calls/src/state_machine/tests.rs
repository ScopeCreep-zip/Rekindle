//! Tests for [`super::CallStateMachine`] and its interpreter.

use std::sync::Arc;

use rekindle_secrets::sframe::SframeSender;
use rekindle_types::subscription_events::CallEvent;

use super::{
    CallInput, CallStateMachine, CallTimer, Effect, InboundInvite, OutgoingCall, CALL_SUPERSEDED,
    CONNECTION_LOST, GLARE_OTHER_LOSES, GLARE_WE_LOST, INVALID_X25519, TEMP_MUTED_DECLINE,
};
use crate::fresh_keypair;
use crate::signaling::handlers::BUSY;
use crate::state::{CallKind, CallStatus};

const ALICE: &str = "aaaa";
const BOB_KEY: &str = "bbbb";
const MALLORY: &str = "eeee";
const T0: u64 = 1_000_000;

fn start(sm: &mut CallStateMachine, call_id: &str, peer: &str, now_ms: u64) -> Vec<Effect> {
    let (secret, _) = fresh_keypair();
    sm.apply(CallInput::LocalStartCall(OutgoingCall {
        call_id: call_id.into(),
        peer: peer.into(),
        peer_display_name: format!("name-of-{peer}"),
        kind: CallKind::Audio,
        my_x25519_secret: secret,
        media_sender: Arc::new(SframeSender::fresh()),
        video_decode_codecs: vec!["vp9".into()],
        now_ms,
    }))
}

fn invite(call_id: &str, from: &str, temp_muted: bool, now_ms: u64) -> CallInput {
    let (_, peer_pub) = fresh_keypair();
    CallInput::InviteReceived(InboundInvite {
        call_id: call_id.into(),
        from: from.into(),
        from_display_name: format!("name-of-{from}"),
        kind: CallKind::Video,
        peer_x25519_pub: peer_pub.to_vec(),
        media_sender: Arc::new(SframeSender::fresh()),
        expires_at_ms: now_ms + 30_000,
        video_decode_codecs: vec!["h264".into(), "vp8".into()],
        temp_muted,
        now_ms,
    })
}

fn accept_from(call_id: &str, from: &str, now_ms: u64) -> CallInput {
    let (_, peer_pub) = fresh_keypair();
    CallInput::AcceptReceived {
        call_id: call_id.into(),
        from: from.into(),
        peer_x25519_pub: peer_pub.to_vec(),
        video_decode_codecs: vec!["vp8".into()],
        now_ms,
    }
}

fn local_accept(call_id: &str, now_ms: u64) -> CallInput {
    let (secret, _) = fresh_keypair();
    CallInput::LocalAccept {
        call_id: call_id.into(),
        my_x25519_secret: secret,
        video_decode_codecs: vec!["vp9".into()],
        now_ms,
    }
}

fn up(call_id: &str, now_ms: u64) -> CallInput {
    CallInput::VoiceTransportUp {
        call_id: call_id.into(),
        now_ms,
    }
}

/// A caller-side call with BOB_KEY, Active since `T0`.
fn active_with_bob(call_id: &str) -> CallStateMachine {
    let mut sm = CallStateMachine::new(ALICE);
    start(&mut sm, call_id, BOB_KEY, T0);
    sm.apply(accept_from(call_id, BOB_KEY, T0));
    sm.apply(up(call_id, T0));
    assert!(matches!(
        sm.get(call_id).map(|c| c.status),
        Some(CallStatus::Active)
    ));
    sm
}

fn emitted(effects: &[Effect]) -> Vec<&CallEvent> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Emit(ev) => Some(ev),
            _ => None,
        })
        .collect()
}

fn has_end_sent(effects: &[Effect], call_id: &str) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::SendCallEnd { call_id: c, .. } if c == call_id))
}

fn declined_with(effects: &[Effect], reason: &str) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::SendCallDecline { reason: r, .. } if r == reason))
}

fn rings(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::SendCallRinging { .. }))
}

#[test]
fn glare_resolves_to_one_call() {
    let mut alice = CallStateMachine::new(ALICE);
    let mut bob = CallStateMachine::new(BOB_KEY);
    start(&mut alice, "ca", BOB_KEY, T0);
    start(&mut bob, "cb", ALICE, T0);
    // ALICE's key is lower: she keeps her outgoing call and declines Bob's.
    let a = alice.apply(invite("cb", BOB_KEY, false, T0));
    assert!(declined_with(&a, GLARE_OTHER_LOSES));
    // BOB_KEY loses: his outgoing call ends locally (CallEnd follows), and
    // Alice's invite rings.
    let b = bob.apply(invite("ca", ALICE, false, T0));
    assert!(has_end_sent(&b, "cb"));
    assert!(rings(&b));
    // The glare loser's CallEnd and the decline reach calls nobody holds.
    assert!(alice
        .apply(CallInput::EndReceived {
            call_id: "cb".into(),
            from: BOB_KEY.into(),
            reason: GLARE_WE_LOST.into(),
            now_ms: T0,
        })
        .is_empty());
    assert!(bob
        .apply(CallInput::DeclineReceived {
            call_id: "cb".into(),
            from: ALICE.into(),
            reason: GLARE_OTHER_LOSES.into(),
            now_ms: T0,
        })
        .is_empty());
    let ids = |sm: &CallStateMachine| -> Vec<String> {
        sm.snapshot().iter().map(|c| c.call_id.clone()).collect()
    };
    assert_eq!(ids(&alice), ["ca"]);
    assert_eq!(ids(&bob), ["ca"]);
}

#[test]
fn a_forged_end_or_decline_from_a_third_key_is_ignored() {
    let mut sm = CallStateMachine::new(ALICE);
    start(&mut sm, "c1", BOB_KEY, T0);
    let forged_decline = sm.apply(CallInput::DeclineReceived {
        call_id: "c1".into(),
        from: MALLORY.into(),
        reason: "nope".into(),
        now_ms: T0,
    });
    assert!(forged_decline.is_empty(), "{forged_decline:?}");
    let forged_ringing = sm.apply(CallInput::RingingReceived {
        call_id: "c1".into(),
        from: MALLORY.into(),
        now_ms: T0,
    });
    assert!(forged_ringing.is_empty());
    let mut active = active_with_bob("c2");
    let forged_end = active.apply(CallInput::EndReceived {
        call_id: "c2".into(),
        from: MALLORY.into(),
        reason: "hangup".into(),
        now_ms: T0,
    });
    assert!(forged_end.is_empty(), "{forged_end:?}");
    assert!(sm.get("c1").is_some() && active.get("c2").is_some());
    // The real peer's End does end it, and only its own media stops.
    let real = active.apply(CallInput::EndReceived {
        call_id: "c2".into(),
        from: BOB_KEY.into(),
        reason: "hangup".into(),
        now_ms: T0,
    });
    assert!(real
        .iter()
        .any(|e| matches!(e, Effect::StopVoiceSession { call_id, .. } if call_id == "c2")));
    assert!(active.get("c2").is_none());
}

#[test]
fn a_temp_muted_caller_is_declined_without_a_ring() {
    let mut sm = CallStateMachine::new(BOB_KEY);
    let effects = sm.apply(invite("c1", ALICE, true, T0));
    assert_eq!(effects.len(), 1, "{effects:?}");
    assert!(declined_with(&effects, TEMP_MUTED_DECLINE));
    assert!(!rings(&effects) && sm.get("c1").is_none());
}

#[test]
fn a_malformed_invite_is_declined() {
    let mut sm = CallStateMachine::new(BOB_KEY);
    let effects = sm.apply(CallInput::InviteReceived(InboundInvite {
        call_id: "c1".into(),
        from: ALICE.into(),
        from_display_name: String::new(),
        kind: CallKind::Audio,
        peer_x25519_pub: vec![0; 16],
        media_sender: Arc::new(SframeSender::fresh()),
        expires_at_ms: T0 + 30_000,
        video_decode_codecs: Vec::new(),
        temp_muted: false,
        now_ms: T0,
    }));
    assert!(declined_with(&effects, INVALID_X25519));
    assert!(sm.get("c1").is_none());
}

#[test]
fn a_duplicate_invite_re_rings_and_an_ended_one_never_rings() {
    let mut sm = CallStateMachine::new(BOB_KEY);
    let first = sm.apply(invite("c1", ALICE, false, T0));
    assert!(rings(&first));
    assert!(emitted(&first)
        .iter()
        .any(|e| matches!(e, CallEvent::Incoming { .. })));
    let again = sm.apply(invite("c1", ALICE, false, T0 + 1_000));
    assert!(rings(&again), "a repeated invite re-sends Ringing");
    assert!(
        emitted(&again).is_empty(),
        "it does not ring the user again"
    );
    sm.apply(CallInput::LocalDecline {
        call_id: "c1".into(),
        reason: "no".into(),
        now_ms: T0 + 2_000,
    });
    let resent = sm.apply(invite("c1", ALICE, false, T0 + 3_000));
    assert!(resent.is_empty(), "{resent:?}");
}

#[test]
fn an_invite_during_a_call_with_another_peer_is_busy() {
    let mut sm = active_with_bob("c1");
    let effects = sm.apply(invite("c2", MALLORY, false, T0));
    assert!(declined_with(&effects, BUSY));
    assert!(sm.get("c2").is_none() && sm.get("c1").is_some());
}

#[test]
fn a_new_invite_from_the_same_peer_supersedes_the_stale_call() {
    let mut sm = active_with_bob("c1");
    let effects = sm.apply(invite("c2", BOB_KEY, false, T0 + 5_000));
    let ended = emitted(&effects).into_iter().any(|e| {
        matches!(e, CallEvent::Ended { call_id, reason } if call_id == "c1" && reason == CALL_SUPERSEDED)
    });
    assert!(ended, "{effects:?}");
    assert!(has_end_sent(&effects, "c1"));
    // The stale call's media stops before the new call rings.
    let stop = effects
        .iter()
        .position(|e| matches!(e, Effect::StopVoiceSession { call_id, .. } if call_id == "c1"));
    let ring = effects
        .iter()
        .position(|e| matches!(e, Effect::SendCallRinging { .. }));
    assert!(stop.is_some() && stop < ring);
    assert!(sm.get("c1").is_none());
    assert!(matches!(
        sm.get("c2").map(|c| c.status),
        Some(CallStatus::Incoming)
    ));
}

#[test]
fn ring_timers_fire_only_at_expiry_and_only_unanswered() {
    let mut sm = CallStateMachine::new(ALICE);
    let effects = start(&mut sm, "c1", BOB_KEY, T0);
    assert!(effects.iter().any(|e| matches!(e,
        Effect::ArmTimer { timer: CallTimer::Dialing, at_ms, .. } if *at_ms == T0 + 30_000)));
    let early = CallInput::LocalDialingTimeout {
        call_id: "c1".into(),
        now_ms: T0 + 10_000,
    };
    assert!(sm.apply(early).is_empty());
    let due = sm.apply(CallInput::LocalDialingTimeout {
        call_id: "c1".into(),
        now_ms: T0 + 30_000,
    });
    assert!(due
        .iter()
        .any(|e| matches!(e, Effect::PersistMissedCall { .. })));
    assert!(matches!(emitted(&due)[..], [CallEvent::TimedOut { .. }]));

    let mut callee = CallStateMachine::new(BOB_KEY);
    callee.apply(invite("c2", ALICE, false, T0));
    let missed = callee.apply(CallInput::LocalIncomingTimeout {
        call_id: "c2".into(),
        now_ms: T0 + 30_000,
    });
    assert!(matches!(emitted(&missed)[..], [CallEvent::Missed { from, .. }] if from == ALICE));
    callee.apply(invite("c3", ALICE, false, T0));
    callee.apply(local_accept("c3", T0 + 1_000));
    let late = callee.apply(CallInput::LocalIncomingTimeout {
        call_id: "c3".into(),
        now_ms: T0 + 30_000,
    });
    assert!(late.is_empty(), "an answered call never times out");
}

#[test]
fn end_all_ends_every_call_locally_first() {
    let mut sm = active_with_bob("c1");
    start(&mut sm, "c2", MALLORY, T0);
    let effects = sm.apply(CallInput::EndAll {
        reason: "logout".into(),
        now_ms: T0,
    });
    assert!(sm.snapshot().is_empty());
    for id in ["c1", "c2"] {
        let ended = effects.iter().position(
            |e| matches!(e, Effect::Emit(CallEvent::Ended { call_id, .. }) if call_id == id),
        );
        let sent = effects
            .iter()
            .position(|e| matches!(e, Effect::SendCallEnd { call_id, .. } if call_id == id));
        assert!(
            ended.is_some() && ended < sent,
            "{id}: local end before CallEnd"
        );
    }
    assert!(effects.iter().any(
        |e| matches!(e, Effect::SettleInvite { call_id, answered: false } if call_id == "c2")
    ));
}

#[test]
fn decode_codecs_are_stored_from_the_invite_and_the_accept() {
    let mut callee = CallStateMachine::new(BOB_KEY);
    callee.apply(invite("c1", ALICE, false, T0));
    let held = callee.get("c1").map(|c| c.peer_video_decode_codecs.clone());
    assert_eq!(held, Some(vec!["h264".to_string(), "vp8".to_string()]));
    let caller = active_with_bob("c2");
    let held = caller.get("c2").map(|c| c.peer_video_decode_codecs.clone());
    assert_eq!(held, Some(vec!["vp8".to_string()]));
}

#[test]
fn consent_expiry_ends_the_call() {
    let mut sm = active_with_bob("c1");
    let live = sm.apply(CallInput::ConsentCheck {
        call_id: "c1".into(),
        last_inbound_media_ms: Some(T0 + 20_000),
        now_ms: T0 + 30_000,
    });
    assert!(
        matches!(live[..], [Effect::ArmTimer { timer: CallTimer::Consent, at_ms, .. }]
        if at_ms == T0 + 50_000)
    );
    let expired = sm.apply(CallInput::ConsentCheck {
        call_id: "c1".into(),
        last_inbound_media_ms: Some(T0 + 20_000),
        now_ms: T0 + 50_000,
    });
    assert!(emitted(&expired)
        .iter()
        .any(|e| matches!(e, CallEvent::Ended { reason, .. } if reason == CONNECTION_LOST)));
    assert!(has_end_sent(&expired, "c1"));
    assert!(sm.get("c1").is_none());
}

#[test]
fn the_callee_accepts_once_its_media_is_up() {
    let mut sm = CallStateMachine::new(BOB_KEY);
    sm.apply(invite("c1", ALICE, false, T0));
    let accepted = sm.apply(local_accept("c1", T0 + 1_000));
    assert!(accepted
        .iter()
        .any(|e| matches!(e, Effect::StartVoiceSession { .. })));
    assert!(!accepted
        .iter()
        .any(|e| matches!(e, Effect::SendCallAccept { .. })));
    let up = sm.apply(up("c1", T0 + 1_500));
    assert!(
        matches!(up.first(), Some(Effect::SendCallAccept { video_decode_codecs, .. })
        if video_decode_codecs == &["vp9".to_string()])
    );
    assert!(up
        .iter()
        .any(|e| matches!(e, Effect::PresentActiveCall { .. })));
    assert!(up.iter().any(
        |e| matches!(e, Effect::FocusConversation { reason, .. } if reason == "call-accepted")
    ));
}

#[test]
fn a_callee_whose_media_fails_declines() {
    let mut sm = CallStateMachine::new(BOB_KEY);
    sm.apply(invite("c1", ALICE, false, T0));
    sm.apply(local_accept("c1", T0));
    let down = sm.apply(CallInput::VoiceTransportDown {
        call_id: "c1".into(),
        reason: "no mic".into(),
        now_ms: T0,
    });
    assert!(declined_with(&down, "voice session failed: no mic"));
    assert!(sm.get("c1").is_none());
}

#[test]
fn media_state_and_reactions_flow_only_with_the_peer() {
    let mut sm = active_with_bob("c1");
    let sent = sm.apply(CallInput::LocalMediaState {
        call_id: "c1".into(),
        audio: true,
        video: false,
        screen: true,
        now_ms: T0,
    });
    assert!(matches!(
        sent[..],
        [Effect::SendCallMediaState { screen: true, .. }]
    ));
    let forged = sm.apply(CallInput::ReactionReceived {
        call_id: "c1".into(),
        from: MALLORY.into(),
        emoji: "x".into(),
        timestamp_ms: 1,
        now_ms: T0,
    });
    assert!(forged.is_empty());
    let real = sm.apply(CallInput::MediaStateReceived {
        call_id: "c1".into(),
        from: BOB_KEY.into(),
        audio: false,
        video: true,
        screen: false,
        timestamp_ms: 7,
        now_ms: T0,
    });
    assert!(matches!(
        emitted(&real)[..],
        [CallEvent::MediaStateChanged { video: true, .. }]
    ));
}

mod port_tests;
