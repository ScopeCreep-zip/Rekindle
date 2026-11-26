//! A window's Leave and Cancel end the call that window is in (plan E4.1):
//! a group window's through `CallOp::LeaveGroup`, a chat window's 1:1
//! through `CallOp::Leave`; only elsewhere is Leave the community voice
//! channel's `VoiceLeave`. Each is enabled by the same call registry the
//! action reads (`signals::Inputs`).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::{Capability as C, SkinAction as A};
use rekindle_types::dto::skin_run::{CallOp, ShellOp, SkinRun, SkinSelection};
use rekindle_types::subscription_events::{CallEvent, DirectCallInfo, SubscriptionEvent};

use super::actions::{map, Mapped, Selected as Selection};
use super::capability::{allows, Group, Selected, SkinWorld};
use super::reads::{in_call, InCall, Reads};
use super::signals::Inputs;

fn group_world() -> SkinWorld {
    let mut w = SkinWorld::default();
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 2,
            creator: true,
            lobby: None,
        },
    );
    w
}

fn group_sel() -> SkinSelection {
    SkinSelection::Group {
        group_id: "g1".into(),
    }
}

/// `in_call` over the registry `inputs`, as `reads::load` builds it.
fn reads_for(inputs: &Inputs, window: &str, action: &A, sel: Option<SkinSelection>) -> Reads {
    Reads {
        in_call: in_call(
            window,
            action,
            sel,
            |g| inputs.our_group_call(g),
            |p| inputs.call_in_with(p),
        ),
        ..Reads::default()
    }
}

fn op(r: Result<Mapped, IpcResponse>) -> Option<CallOp> {
    match r {
        Ok(Mapped::Run(SkinRun::Shell(ShellOp::Call(op)))) => Some(op),
        _ => None,
    }
}

fn connected(call_id: &str, group: Option<&str>, peer: Option<&str>) -> SubscriptionEvent {
    SubscriptionEvent::Call(CallEvent::Connected {
        call_id: call_id.into(),
        group_id: group.map(str::to_owned),
        direct: peer.map(|p| DirectCallInfo {
            kind: "audio".into(),
            peer_key: p.into(),
            peer_display_name: "Bob".into(),
            expected_local_camera: false,
        }),
    })
}

#[test]
fn a_group_windows_leave_leaves_the_group_call_we_are_in() {
    let mut i = Inputs::default();
    let w = group_world();
    let g1 = Selected::Group { group_id: "g1" };
    i.fold(&SubscriptionEvent::Call(CallEvent::Incoming {
        call_id: "gc1".into(),
        from: "b".into(),
        display_name: "B".into(),
        kind: "audio".into(),
        participants: vec!["b".into(), "me".into()],
        is_group: true,
        expires_at_ms: 0,
        group_id: Some("g1".into()),
    }));
    // Ringing here: not ours to leave yet.
    assert!(!allows(C::LeaveVoiceChat, &i, &w, g1));
    let r = reads_for(&i, "chat-gg1", &A::LeaveGroupVoiceChat, Some(group_sel()));
    assert!(matches!(
        map(&A::LeaveGroupVoiceChat, Selection::default(), &r),
        Err(IpcResponse::Error { code: 409, .. })
    ));
    i.fold(&connected("gc1", Some("g1"), None));
    assert!(allows(C::LeaveVoiceChat, &i, &w, g1));
    for action in [A::LeaveGroupVoiceChat, A::LeaveVoiceChat] {
        let r = reads_for(&i, "chat-gg1", &action, Some(group_sel()));
        assert_eq!(
            op(map(&action, Selection::default(), &r)),
            Some(CallOp::LeaveGroup {
                group_id: "g1".into(),
                call_id: "gc1".into(),
            }),
            "{action:?}"
        );
    }
}

#[test]
fn a_chat_windows_leave_leaves_its_one_to_one_call() {
    let mut i = Inputs::default();
    i.fold(&SubscriptionEvent::Call(CallEvent::Started {
        call_id: "c1".into(),
        kind: "audio".into(),
        peer_key: "bob".into(),
        peer_display_name: "Bob".into(),
        expires_at_ms: 0,
    }));
    i.fold(&connected("c1", None, Some("bob")));
    let bob = Some(SkinSelection::Friend {
        public_key: "bob".into(),
    });
    let sel = Selection {
        friend: Some("bob"),
        ..Selection::default()
    };
    let r = reads_for(&i, "chat-bob", &A::LeaveVoiceChat, bob.clone());
    assert_eq!(
        r.in_call,
        Some(InCall::Direct {
            call_id: Some("c1".into())
        })
    );
    assert_eq!(
        op(map(&A::LeaveVoiceChat, sel, &r)),
        Some(CallOp::Leave {
            call_id: "c1".into()
        })
    );
    // A 1:1 window's Cancel stays the shell's End of its own ring.
    let r = reads_for(&i, "chat-bob", &A::CancelVoiceChat, bob);
    assert_eq!(op(map(&A::CancelVoiceChat, sel, &r)), Some(CallOp::End));
}

#[test]
fn a_community_voice_windows_leave_stays_voice_leave() {
    let i = Inputs::default();
    let r = reads_for(&i, "community-c1", &A::LeaveVoiceChat, None);
    assert_eq!(r.in_call, None);
    assert!(matches!(
        map(&A::LeaveVoiceChat, Selection::default(), &r),
        Ok(Mapped::Request(ref req)) if matches!(**req, IpcRequest::VoiceLeave)
    ));
}

#[test]
fn cancel_ends_our_own_group_ring() {
    let mut i = Inputs::default();
    let w = group_world();
    let g1 = Selected::Group { group_id: "g1" };
    assert!(!allows(C::CancelVoiceChat, &i, &w, g1));
    i.fold(&SubscriptionEvent::Call(CallEvent::GroupStarted {
        call_id: "gc2".into(),
        kind: "audio".into(),
        participants: vec!["me".into(), "b".into(), "c".into()],
        group_id: Some("g1".into()),
        expires_at_ms: 0,
    }));
    assert!(allows(C::CancelVoiceChat, &i, &w, g1));
    assert!(!allows(C::LeaveVoiceChat, &i, &w, g1));
    assert!(!allows(C::HostVoiceChat, &i, &w, g1));
    assert!(!allows(C::VoiceChat, &i, &w, g1));
    let r = reads_for(&i, "chat-gg1", &A::CancelVoiceChat, Some(group_sel()));
    assert_eq!(
        op(map(&A::CancelVoiceChat, Selection::default(), &r)),
        Some(CallOp::LeaveGroup {
            group_id: "g1".into(),
            call_id: "gc2".into(),
        })
    );
    // The first accept: our ring is now a call we are in.
    i.fold(&connected("gc2", Some("g1"), None));
    assert!(!allows(C::CancelVoiceChat, &i, &w, g1));
    assert!(allows(C::LeaveVoiceChat, &i, &w, g1));
}
