//! A community window's pack row (plan E7.7 HSC1): Call, Mute, Join and
//! Profile pressed on a window whose selection is a community resolve to
//! the community's operations, and each is refused with the Host's words
//! (CR1) when it cannot run.

use std::collections::BTreeMap;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::skin_action::{Capability as C, SkinAction as A};
use rekindle_types::dto::skin_run::{CallOp, ShellOp, SkinRun, SkinSelection};
use rekindle_types::game_server::JoinSource;
use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent, VoiceScope};

use super::actions::{map, Mapped, Selected as Selection};
use super::capability::{refusal, Selected, SkinWorld};
use super::community::{
    read, CommunityServer, SkinCommunity, IN_VOICE, NOT_IN_VOICE, NOT_MEMBER, NO_SERVER, NO_VOICE,
};
use super::reads::Reads;
use super::signals::Inputs;

fn server(id: &str, address: &str) -> CommunityServer {
    CommunityServer {
        id: id.into(),
        game_id: Some(4100),
        address: address.into(),
        not_joinable: None,
    }
}

/// `c1` holds voice channels `v1` and `v2` and game servers `s1` and
/// `s2`; `c0` holds neither.
fn views() -> BTreeMap<String, SkinCommunity> {
    let mut v = BTreeMap::new();
    v.insert(
        "c1".to_owned(),
        SkinCommunity {
            voice: vec!["v1".into(), "v2".into()],
            servers: vec![
                server("s1", "198.51.100.1:27015"),
                server("s2", "198.51.100.2:27016"),
            ],
        },
    );
    v.insert("c0".to_owned(), SkinCommunity::default());
    v
}

fn sel(community: &str, channel: Option<&str>) -> SkinSelection {
    sel_server(community, channel, None)
}

fn sel_server(community: &str, channel: Option<&str>, server: Option<&str>) -> SkinSelection {
    SkinSelection::Community {
        community: community.into(),
        channel: channel.map(str::to_owned),
        server: server.map(str::to_owned),
    }
}

/// The press launched `address` by the one launch path, its source the
/// community.
fn launched(r: Result<Mapped, IpcResponse>) -> String {
    let Ok(Mapped::Request(req)) = r else {
        panic!("not a request: {r:?}");
    };
    match *req {
        IpcRequest::GameLaunch {
            game_id: 4100,
            server: Some(server),
            source: JoinSource::Community,
        } => server.to_string(),
        other => panic!("not the community launch: {other:?}"),
    }
}

/// The press of `action` on a window with `selection`, its reads built as
/// `reads::load` builds them.
fn press(action: &A, selection: &SkinSelection) -> Result<Mapped, IpcResponse> {
    let views = views();
    let reads = Reads {
        community: read(Some(selection), |id| views.get(id).cloned()),
        ..Reads::default()
    };
    map(action, Selection::default(), &reads)
}

fn shell_op(r: Result<Mapped, IpcResponse>) -> ShellOp {
    match r {
        Ok(Mapped::Run(SkinRun::Shell(op))) => op,
        other => panic!("not a shell op: {other:?}"),
    }
}

fn refused_with(r: Result<Mapped, IpcResponse>) -> String {
    match r {
        Err(IpcResponse::Error {
            code: 409, message, ..
        }) => message,
        other => panic!("not a refusal: {other:?}"),
    }
}

fn world() -> SkinWorld {
    SkinWorld {
        community_views: views(),
        ..SkinWorld::default()
    }
}

fn joined(community: &str, channel: &str) -> SubscriptionEvent {
    SubscriptionEvent::Voice(VoiceEvent::LocalJoined {
        scope: VoiceScope::Community {
            community: community.into(),
            channel: channel.into(),
        },
    })
}

#[test]
fn call_joins_the_selected_voice_channel_else_the_first() {
    let join = |community: &str, channel: &str| {
        ShellOp::Call(CallOp::JoinVoice {
            community: community.into(),
            channel: channel.into(),
        })
    };
    for action in [A::VoiceChat, A::HostVoiceChat] {
        // A text channel shown: the community's first voice channel.
        assert_eq!(
            shell_op(press(&action, &sel("c1", Some("t1")))),
            join("c1", "v1")
        );
        assert_eq!(shell_op(press(&action, &sel("c1", None))), join("c1", "v1"));
        // A voice channel shown: that channel.
        assert_eq!(
            shell_op(press(&action, &sel("c1", Some("v2")))),
            join("c1", "v2")
        );
        assert_eq!(refused_with(press(&action, &sel("c0", None))), NO_VOICE);
        assert_eq!(refused_with(press(&action, &sel("cx", None))), NOT_MEMBER);
    }
}

#[test]
fn mute_join_and_profile_presses_resolve_on_a_community_window() {
    let s = sel("c1", Some("v1"));
    assert_eq!(
        shell_op(press(&A::MuteSelf, &s)),
        ShellOp::Call(CallOp::Mute)
    );
    assert_eq!(
        shell_op(press(&A::UnmuteSelf, &s)),
        ShellOp::Call(CallOp::Unmute)
    );
    for action in [A::ViewProfile, A::ProfileFriend] {
        assert_eq!(
            shell_op(press(&action, &s)),
            ShellOp::OpenCommunityInfo {
                community: "c1".into()
            }
        );
        assert_eq!(refused_with(press(&action, &sel("cx", None))), NOT_MEMBER);
    }
}

#[test]
fn join_launches_the_selected_server_else_the_first_listed() {
    let first = launched(press(&A::JoinGameLobby, &sel("c1", Some("v1"))));
    assert_eq!(first, "198.51.100.1:27015");
    let chosen = launched(press(
        &A::JoinGameLobby,
        &sel_server("c1", None, Some("s2")),
    ));
    assert_eq!(chosen, "198.51.100.2:27016");
    // A server id the community no longer lists: its first.
    let gone = launched(press(
        &A::JoinGameLobby,
        &sel_server("c1", None, Some("sx")),
    ));
    assert_eq!(gone, "198.51.100.1:27015");
    assert_eq!(
        refused_with(press(&A::JoinGameLobby, &sel("c0", None))),
        NO_SERVER
    );
    assert_eq!(
        refused_with(press(&A::JoinGameLobby, &sel("cx", None))),
        NOT_MEMBER
    );
    assert_eq!(NO_SERVER, "This community lists no game server to join.");
    // A pack's plain Join node (`JoinFriend`, Symbiosis) is the same press.
    let plain = launched(press(&A::JoinFriend, &sel("c1", Some("v1"))));
    assert_eq!(plain, "198.51.100.1:27015");
    assert_eq!(
        refused_with(press(&A::JoinFriend, &sel("c0", None))),
        NO_SERVER
    );
}

#[test]
fn each_community_capability_is_refused_with_the_hosts_words() {
    let (w, mut i) = (world(), Inputs::default());
    let c1 = sel("c1", Some("v1"));
    let c0 = sel("c0", None);
    let cx = sel("cx", None);
    let on = |s: &SkinSelection, c: C, i: &Inputs| refusal(c, i, &w, Selected::of(Some(s), None));
    // No voice channel: Call is refused with its reason.
    assert_eq!(on(&c0, C::VoiceChat, &i).as_deref(), Some(NO_VOICE));
    assert_eq!(on(&c0, C::HostVoiceChat, &i).as_deref(), Some(NO_VOICE));
    assert_eq!(on(&cx, C::VoiceChat, &i).as_deref(), Some(NOT_MEMBER));
    assert_eq!(on(&c1, C::VoiceChat, &i), None);
    // Not in voice: Mute and Leave are refused.
    for c in [C::MuteSelf, C::UnmuteSelf, C::LeaveVoiceChat] {
        let why = on(&c1, c, &i).unwrap_or_else(|| panic!("{c:?} holds"));
        assert!(!why.is_empty(), "{c:?} has no reason");
    }
    // Join holds while the community lists a server, else its reason.
    assert_eq!(on(&c1, C::JoinGameLobby, &i), None);
    assert_eq!(on(&c0, C::JoinGameLobby, &i).as_deref(), Some(NO_SERVER));
    assert_eq!(on(&cx, C::JoinGameLobby, &i).as_deref(), Some(NOT_MEMBER));
    // A plain Join node asks `Capability::Join`: the same verdict.
    assert_eq!(on(&c1, C::Join, &i), None);
    assert_eq!(on(&c0, C::Join, &i).as_deref(), Some(NO_SERVER));
    assert_eq!(on(&cx, C::Join, &i).as_deref(), Some(NOT_MEMBER));
    assert_eq!(
        on(&c1, C::LeaveVoiceChat, &i).as_deref(),
        Some(NOT_IN_VOICE)
    );
    assert_eq!(on(&c1, C::Profile, &i), None);
    assert_eq!(on(&cx, C::Profile, &i).as_deref(), Some(NOT_MEMBER));
    // In `v1`: Call there is the opposite op's (Leave), Mute holds.
    assert!(i.fold(&joined("c1", "v1")));
    assert_eq!(on(&c1, C::VoiceChat, &i).as_deref(), Some(IN_VOICE));
    assert_eq!(on(&c1, C::LeaveVoiceChat, &i), None);
    assert_eq!(on(&c1, C::MuteSelf, &i), None);
    assert!(on(&c1, C::UnmuteSelf, &i).is_some_and(|w| !w.is_empty()));
    // Another voice channel of it can still be joined.
    assert_eq!(on(&sel("c1", Some("v2")), C::VoiceChat, &i), None);
}
