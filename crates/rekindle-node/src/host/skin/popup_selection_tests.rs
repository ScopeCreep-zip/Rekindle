//! A popup's selection is its launcher's (plan E7.7 HSC1): Alloy's info
//! view (`popup-buddy-list-Info…`, INFO→ from the buddy list) shows the
//! buddy list's selection, so its buttons are judged
//! (`signals::window_sets`) and pressed (`actions::resolve` with
//! `reads::load`'s community read) on that selection, in the community's
//! words; a window that is not a popup keeps its own.

use std::collections::BTreeMap;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::{Capability as C, SkinAction as A};
use rekindle_types::dto::skin_run::SkinSelection;
use rekindle_types::dto::skin_signals::{DisableCause, SkinSignalSet};
use rekindle_types::game_server::JoinSource;

use super::actions::{resolve, Resolved};
use super::capability::SkinWorld;
use super::community::{read, CommunityServer, SkinCommunity, NO_SERVER, NO_VOICE};
use super::reads::Reads;
use super::signals::{window_sets, Inputs};
use super::SkinState;

const LAUNCHER: &str = "buddy-list";
const POPUP: &str = "popup-buddy-list-Info";

/// `c1` lists a voice channel and a game server; `c0` lists neither.
fn views() -> BTreeMap<String, SkinCommunity> {
    let mut v = BTreeMap::new();
    v.insert(
        "c1".to_owned(),
        SkinCommunity {
            voice: vec!["v1".into()],
            servers: vec![CommunityServer {
                id: "s1".into(),
                game_id: Some(4100),
                address: "198.51.100.1:27015".into(),
                not_joinable: None,
            }],
        },
    );
    v.insert("c0".to_owned(), SkinCommunity::default());
    v
}

fn community(id: &str) -> SkinSelection {
    SkinSelection::Community {
        community: id.into(),
        channel: None,
        server: None,
    }
}

/// A skin state with `POPUP` open, launched from `LAUNCHER`.
fn with_popup() -> (SkinState, Inputs) {
    let skin = SkinState::default();
    skin.launchers.set(POPUP, Some(LAUNCHER));
    let mut inputs = Inputs::default();
    inputs.open(LAUNCHER);
    inputs.open(POPUP);
    (skin, inputs)
}

fn sets(skin: &SkinState, inputs: &Inputs) -> BTreeMap<String, SkinSignalSet> {
    let world = SkinWorld {
        community_views: views(),
        ..SkinWorld::default()
    };
    window_sets(inputs, &skin.selection, &skin.launchers, &world, |_| None)
}

fn reason(set: &SkinSignalSet, c: C) -> Option<&str> {
    set.disabled_reasons
        .get(&DisableCause::Capability(c))
        .map(String::as_str)
}

/// `action` pressed in `window`, its reads built as `reads::load` builds
/// the community read (from the window's selection window).
fn press(skin: &SkinState, window: &str, action: A) -> Result<Resolved, IpcResponse> {
    let views = views();
    let shown = skin.selection.get(&skin.launchers.selection_window(window));
    let reads = Reads {
        community: read(shown.as_ref(), |id| views.get(id).cloned()),
        ..Reads::default()
    };
    let request = IpcRequest::RunSkinAction {
        window: window.into(),
        node: "JoinButton".into(),
        action,
    };
    resolve(skin, &reads, request)
}

#[test]
fn a_popup_set_is_refused_in_its_launchers_community_words() {
    let (skin, inputs) = with_popup();
    skin.selection.set(LAUNCHER, Some(community("c0")));
    let all = sets(&skin, &inputs);
    let popup = &all[POPUP];
    for c in [C::Join, C::JoinGameLobby] {
        assert!(popup.cannot.contains(&c), "{c:?} not refused");
        assert_eq!(reason(popup, c), Some(NO_SERVER), "{c:?}");
    }
    assert!(popup.cannot.contains(&C::HostVoiceChat));
    assert_eq!(reason(popup, C::HostVoiceChat), Some(NO_VOICE));
    // The popup's set is its launcher's.
    assert_eq!(popup.cannot, all[LAUNCHER].cannot);
}

#[test]
fn a_popups_join_friend_press_launches_its_launchers_community_server() {
    let (skin, _) = with_popup();
    skin.selection.set(LAUNCHER, Some(community("c1")));
    match press(&skin, POPUP, A::JoinFriend) {
        Ok(Resolved::Request(IpcRequest::GameLaunch {
            game_id: 4100,
            server: Some(server),
            source: JoinSource::Community,
        })) => assert_eq!(server.to_string(), "198.51.100.1:27015"),
        other => panic!("not the community launch: {other:?}"),
    }
    skin.selection.set(LAUNCHER, Some(community("c0")));
    match press(&skin, POPUP, A::JoinFriend) {
        Err(IpcResponse::Error {
            code: 409, message, ..
        }) => assert_eq!(message, NO_SERVER),
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn a_launchers_selection_change_recomputes_its_popups_set() {
    let (skin, inputs) = with_popup();
    skin.selection.set(LAUNCHER, Some(community("c1")));
    let before = sets(&skin, &inputs);
    assert!(before[POPUP].can.contains(&C::Join));
    assert_eq!(reason(&before[POPUP], C::Join), None);
    skin.selection.set(LAUNCHER, Some(community("c0")));
    let after = sets(&skin, &inputs);
    assert!(after[POPUP].cannot.contains(&C::Join));
    assert_eq!(reason(&after[POPUP], C::Join), Some(NO_SERVER));
}

#[test]
fn a_window_that_is_not_a_popup_keeps_its_own_selection() {
    let (skin, mut inputs) = with_popup();
    let own = "community-c1";
    inputs.open(own);
    skin.selection.set(LAUNCHER, Some(community("c0")));
    skin.selection.set(own, Some(community("c1")));
    assert_eq!(skin.launchers.selection_window(own), own);
    let all = sets(&skin, &inputs);
    assert!(all[own].can.contains(&C::Join));
    assert!(all[own].can.contains(&C::HostVoiceChat));
    assert!(all[POPUP].cannot.contains(&C::Join));
}
