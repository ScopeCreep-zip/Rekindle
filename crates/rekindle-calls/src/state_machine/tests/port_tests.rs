//! [`super::super::port::interpret`] over a recording port: the effects
//! become port calls, and outcomes feed back as inputs.

use parking_lot::Mutex;

use super::*;
use crate::error::CallError;
use crate::state_machine::port::{interpret, CallPort};
use rekindle_codec::capnp_codec::direct::DirectBody;

#[derive(Default)]
struct Recording {
    calls: Mutex<Vec<String>>,
    events: Mutex<Vec<CallEvent>>,
    refuse_invites: bool,
}

#[async_trait::async_trait]
impl CallPort for Recording {
    fn now_ms(&self) -> u64 {
        T0
    }
    async fn commit_invite(
        &self,
        recipient: &str,
        call_id: &str,
        _body: DirectBody,
        _deadline_ms: u64,
    ) -> Result<(), CallError> {
        self.calls
            .lock()
            .push(format!("invite {recipient} {call_id}"));
        if self.refuse_invites {
            return Err(CallError::Transport("no session".into()));
        }
        Ok(())
    }
    async fn settle_invite(&self, call_id: &str, answered: bool) {
        self.calls
            .lock()
            .push(format!("settle {call_id} {answered}"));
    }
    async fn commit_end(&self, recipient: &str, _: DirectBody, _: u64) -> Result<(), CallError> {
        self.calls.lock().push(format!("end {recipient}"));
        Ok(())
    }
    async fn send(&self, recipient: &str, body: DirectBody) -> Result<(), CallError> {
        let kind = match body {
            DirectBody::CallRinging(_) => "ringing",
            DirectBody::CallAccept(_) => "accept",
            DirectBody::CallDecline(_) => "decline",
            _ => "other",
        };
        self.calls.lock().push(format!("send {kind} {recipient}"));
        Ok(())
    }
    fn arm_timer(&self, call_id: &str, timer: CallTimer, at_ms: u64) {
        self.calls
            .lock()
            .push(format!("arm {call_id} {timer:?} {at_ms}"));
    }
    fn cancel_timers(&self, call_id: &str) {
        self.calls.lock().push(format!("cancel {call_id}"));
    }
    async fn start_media(&self, call_id: &str, _: &str, _: CallKind) -> Result<(), CallError> {
        self.calls.lock().push(format!("start {call_id}"));
        Ok(())
    }
    async fn stop_media(&self, call_id: &str) {
        self.calls.lock().push(format!("stop {call_id}"));
    }
    async fn persist_missed_call(
        &self,
        call_id: &str,
        _: &str,
        _: CallKind,
        _: u64,
    ) -> Result<(), CallError> {
        self.calls.lock().push(format!("missed {call_id}"));
        Ok(())
    }
    fn emit(&self, event: CallEvent) {
        self.events.lock().push(event);
    }
    fn present_active_call(&self, call_id: &str) {
        self.calls.lock().push(format!("present {call_id}"));
    }
    fn surface_call_window(&self, call_id: &str) {
        self.calls.lock().push(format!("surface {call_id}"));
    }
}

/// Apply `input` and perform its effects, feeding outcomes back until none.
async fn drive(sm: &mut CallStateMachine, port: &Recording, input: CallInput) {
    let mut pending = vec![input];
    while let Some(input) = pending.pop() {
        for effect in sm.apply(input) {
            if let Some(next) = interpret(port, effect).await {
                pending.push(next);
            }
        }
    }
}

#[tokio::test]
async fn an_invite_that_cannot_be_committed_is_unreachable() {
    let port = Recording {
        refuse_invites: true,
        ..Recording::default()
    };
    let mut sm = CallStateMachine::new(ALICE);
    let (secret, _) = fresh_keypair();
    let call = OutgoingCall {
        call_id: "c1".into(),
        peer: BOB_KEY.into(),
        peer_display_name: "Bob".into(),
        kind: CallKind::Audio,
        my_x25519_secret: secret,
        video_decode_codecs: Vec::new(),
        now_ms: T0,
    };
    drive(&mut sm, &port, CallInput::LocalStartCall(call)).await;
    assert!(sm.get("c1").is_none());
    let events = port.events.lock();
    assert!(matches!(events.first(), Some(CallEvent::Started { .. })));
    assert!(events.iter().any(
        |e| matches!(e, CallEvent::FocusConversation { reason, .. } if reason == "call-started")
    ));
    assert!(
        matches!(events.last(), Some(CallEvent::Unreachable { call_id, .. }) if call_id == "c1")
    );
    assert!(port.calls.lock().contains(&"settle c1 false".to_string()));
}

#[tokio::test]
async fn an_accepted_invite_brings_media_up_then_sends_the_accept() {
    let port = Recording::default();
    let mut sm = CallStateMachine::new(BOB_KEY);
    drive(&mut sm, &port, invite("c1", ALICE, false, T0)).await;
    drive(&mut sm, &port, local_accept("c1", T0)).await;
    let calls = port.calls.lock().clone();
    let start = calls.iter().position(|c| c == "start c1");
    let accept = calls
        .iter()
        .position(|c| c == &format!("send accept {ALICE}"));
    assert!(start.is_some() && start < accept, "{calls:?}");
    assert!(calls.contains(&"surface c1".to_string()));
    assert!(calls.contains(&"present c1".to_string()));
    assert!(matches!(
        sm.get("c1").map(|c| c.status),
        Some(CallStatus::Active)
    ));
}
