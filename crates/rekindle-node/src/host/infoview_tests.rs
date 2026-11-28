//! The Infoview through the Host (plan E7.15, IV4): each subject composes
//! its sections from Host state, a shown event pushes the open window's
//! view once (coalesced), and a friend's page never lists communities.

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::Caller;
use rekindle_types::dto::infoview::{
    InfoviewAction, InfoviewEvent, InfoviewSection, InfoviewSubject, InfoviewView, JoinSource,
};
use rekindle_types::dto::skin_run::SkinSelection;
use rekindle_types::friend::{GameInfoState, NotJoinableReason};
use rekindle_types::subscription_events::{
    FriendEvent, StreamFrame, SubscriptionEvent, SystemEvent,
};

use super::activity::{seen_of, store};
use super::direct_tests::ann;
use super::game_totals_tests::open_session;
use super::runtime::tests::seed_registry_community;
use super::Host;

async fn get(host: &Host, subject: InfoviewSubject, window: &str) -> InfoviewView {
    let request = IpcRequest::InfoviewGet {
        subject,
        window: window.to_owned(),
    };
    match rekindle_ipc::HostApi::call(host, Caller::shell(), request).await {
        IpcResponse::Ok { data, .. } => serde_json::from_value(data).unwrap(),
        other => panic!("{other:?}"),
    }
}

fn kinds(view: &InfoviewView) -> Vec<String> {
    view.sections
        .iter()
        .map(|s| {
            serde_json::to_value(s).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

/// Ann and her friend Bob, playing on a server he shared.
pub(super) async fn ann_and_bob() -> (Vec<tempfile::TempDir>, std::sync::Arc<Host>, String, String)
{
    let dirs = vec![tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let (host, owner) = ann(&dirs[0]).await;
    let (bob_host, bob) = ann(&dirs[1]).await;
    open_session(&host, owner, &bob_host, bob).await;
    let bob = hex::encode(bob);
    let before = host.ctx.session.read().as_ref().unwrap().friends[&bob].clone();
    let mut after = before.clone();
    after.game_info = Some(GameInfoState {
        game_id: 4_100,
        game_name: "Game".into(),
        server_info: Some("Dust".into()),
        elapsed_seconds: 30,
        server_address: Some("203.0.113.7:27015".into()),
        icon_key: None,
        joinable: true,
        joinable_reason: None,
    });
    host.ctx
        .session
        .write()
        .as_mut()
        .unwrap()
        .friends
        .insert(bob.clone(), after.clone());
    store(
        &host.ctx,
        bob.clone(),
        Some(seen_of(&before, None)),
        seen_of(&after, None),
    )
    .await;
    (dirs, host, hex::encode(owner), bob)
}

#[tokio::test]
async fn own_page_composes_the_feed_our_communities_and_friends_in_game() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    let view = get(&host, InfoviewSubject::Own, "cli").await;
    assert_eq!(kinds(&view), ["activity", "communities", "nowPlaying"]);
    let InfoviewSection::Activity { entries, more } = &view.sections[0] else {
        panic!("{view:?}");
    };
    assert_eq!(entries.len(), 1);
    // `ActivityKind::name` (`rekindle-types/src/activity.rs:67-79`) is the
    // camelCase `activity_log.kind` column: `StartedGame` is "startedGame".
    assert_eq!(entries[0].kind.name(), "startedGame");
    assert!(more.is_none());
    let InfoviewSection::NowPlaying { rows } = &view.sections[2] else {
        panic!("{view:?}");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].friend.public_key, bob);
    assert_eq!(
        serde_json::to_value(&rows[0].join).unwrap(),
        serde_json::json!({
            "type": "joinServer",
            "gameId": 4100,
            "server": "203.0.113.7:27015",
            "source": "friendPresence",
            "joinable": true,
            "joinableReason": null,
        })
    );
}

/// Bob's game as his presence states it: `server`, and the verdict the
/// presence adapter judged for it.
fn set_bobs_game(
    host: &Host,
    bob: &str,
    server: Option<&str>,
    reason: Option<NotJoinableReason>,
) -> GameInfoState {
    let mut session = host.ctx.session.write();
    let game = session
        .as_mut()
        .unwrap()
        .friends
        .get_mut(bob)
        .unwrap()
        .game_info
        .as_mut()
        .unwrap();
    game.server_address = server.map(str::to_owned);
    game.joinable = reason.is_none();
    game.joinable_reason = reason;
    game.clone()
}

/// A friend in game who shared no server still gets `Join`, sourced from
/// their presence and disabled with the presence's reason: the window never
/// decides whether to render it.
#[tokio::test]
async fn a_playing_row_always_joins_from_friend_presence() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    set_bobs_game(&host, &bob, None, Some(NotJoinableReason::NoServer));
    let view = get(&host, InfoviewSubject::Own, "cli").await;
    let InfoviewSection::NowPlaying { rows } = &view.sections[2] else {
        panic!("{view:?}");
    };
    assert_eq!(
        rows[0].join,
        InfoviewAction::JoinServer {
            game_id: Some(4_100),
            server: None,
            source: JoinSource::FriendPresence,
            joinable: false,
            joinable_reason: Some(NotJoinableReason::NoServer),
        }
    );
}

/// A playing row's verdict is the friend's `GameInfoState` verdict, on our
/// own page and on the friend's page alike, in both directions.
#[tokio::test]
async fn a_playing_rows_verdict_matches_the_friends_game_info() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    for reason in [Some(NotJoinableReason::CannotLaunch), None] {
        let game = set_bobs_game(&host, &bob, Some("203.0.113.7:27015"), reason);
        let friend_page = InfoviewSubject::Friend {
            public_key: bob.clone(),
        };
        for subject in [InfoviewSubject::Own, friend_page] {
            let view = get(&host, subject, "cli").await;
            let Some(InfoviewSection::NowPlaying { rows }) = view.sections.last() else {
                panic!("{view:?}");
            };
            let InfoviewAction::JoinServer {
                joinable,
                joinable_reason,
                ..
            } = &rows[0].join
            else {
                panic!("{rows:?}");
            };
            assert_eq!(
                (*joinable, *joinable_reason),
                (game.joinable, game.joinable_reason)
            );
        }
    }
}

#[tokio::test]
async fn a_friends_page_has_header_bio_and_game_and_never_communities() {
    let (_dirs, host, owner, bob) = ann_and_bob().await;
    seed_registry_community(&host, &owner).await;
    let (o, b) = (owner.clone(), bob.clone());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::bio::set_friend(c, &o, &b, Some("hello")))
        .await
        .unwrap();
    let view = get(
        &host,
        InfoviewSubject::Friend {
            public_key: bob.clone(),
        },
        "cli",
    )
    .await;
    assert_eq!(kinds(&view), ["friendHeader", "bio", "nowPlaying"]);
    let json = serde_json::to_string(&view).unwrap();
    assert!(
        !json.contains("community") && !json.contains("communities"),
        "{json}"
    );
    assert!(!json.contains("\"c1\""), "{json}");
}

#[tokio::test]
async fn a_group_page_lists_its_members() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    let view = get(
        &host,
        InfoviewSubject::FriendGroup { group_id: None },
        "cli",
    )
    .await;
    let [InfoviewSection::GroupMembers {
        online, offline, ..
    }] = view.sections.as_slice()
    else {
        panic!("{view:?}");
    };
    let all: Vec<&str> = online
        .iter()
        .chain(offline)
        .map(|f| f.public_key.as_str())
        .collect();
    assert_eq!(all, [bob.as_str()]);
}

/// The Infoview pushes the journal shows: each `(window, subject)` that
/// arrives before `end`, stopping at the first when `first`.
async fn pushes_until(
    stream: &mut rekindle_events::EventStream,
    end: tokio::time::Instant,
    first: bool,
) -> Vec<(String, InfoviewSubject)> {
    let mut pushes = Vec::new();
    while let Ok(Some(frame)) = tokio::time::timeout_at(end, stream.next()).await {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::System(SystemEvent::Infoview(InfoviewEvent::Updated {
                window,
                view,
            })) = s.event
            {
                pushes.push((window, view.subject));
                if first {
                    break;
                }
            }
        }
    }
    pushes
}

/// Two shown events in one period push the open window's view once. The
/// worker reads both on the paused clock, so no time passes between them
/// (a loaded machine cannot split the period); the setup's own shown
/// events are let come due first, while no window is open.
#[tokio::test]
async fn shown_events_push_the_open_window_once() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    tokio::time::pause();
    tokio::time::sleep(Duration::from_secs(1)).await;
    tokio::time::resume();
    let mut stream = host.ctx.journal.subscribe(None);
    get(&host, InfoviewSubject::Own, "main").await;
    tokio::time::pause();
    for nick in ["A", "B"] {
        host.ctx
            .publish_event(SubscriptionEvent::Friend(FriendEvent::NicknameChanged {
                peer_key: bob.clone(),
                nickname: Some(nick.into()),
            }));
    }
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    let now = tokio::time::Instant::now;
    let mut pushes = pushes_until(&mut stream, now() + Duration::from_secs(30), true).await;
    pushes.extend(pushes_until(&mut stream, now() + Duration::from_secs(1), false).await);
    assert_eq!(pushes, [("main".to_owned(), InfoviewSubject::Own)]);
}

/// A community's page: its header with the online count, then its events,
/// pins and game servers. The count is the member list's own rule (finding
/// OC1): us while our own row shows us present, plus every member its beats
/// show present; invisible, we count in neither view.
#[tokio::test]
async fn a_community_page_counts_us_and_the_members_its_beats_show_present() {
    use rekindle_codec::member_record::{BeatState, PresenceBeat};
    use rekindle_types::presence::SessionStatus;
    let (_dirs, host, owner, _bob) = ann_and_bob().await;
    seed_registry_community(&host, &owner).await;
    // A joined community holds our pseudonym and its loaded governance
    // state (warm restore); an empty state grants Ann nothing, so events,
    // pins and servers are empty.
    let me = rekindle_types::id::PseudonymKey([0xaa; 32]);
    let mut governance = rekindle_governance::state::GovernanceState::default();
    governance.members.insert(
        me.clone(),
        rekindle_governance::state::MemberState {
            member_record: "record".into(),
            member_record_owner: "owner".into(),
            joined: rekindle_governance::op::OpStamp::MIN,
        },
    );
    {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        c.my_pseudonym_key = Some("aa".repeat(32));
        c.governance_state = Some(governance);
    }
    host.ctx
        .session
        .write()
        .as_mut()
        .unwrap()
        .communities
        .insert(
            "c1".into(),
            rekindle_transport::CommunityMembership {
                anchor: "c1".into(),
                pseudonym_key: String::new(),
                registry_key: "registry".into(),
                slot_index: 0,
                community_name: "C".into(),
                slot_seed: None,
                segment_index: None,
                mek_generation: 0,
            },
        );
    let roster_owner = owner.clone();
    let my_key = me.to_hex();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::members::insert_if_absent(conn, &roster_owner, "c1", &my_key, 1)
        })
        .await
        .unwrap();
    let read = || async {
        let view = get(
            &host,
            InfoviewSubject::Community {
                community: "c1".into(),
            },
            "community-c1",
        )
        .await;
        assert_eq!(
            kinds(&view),
            ["communityHeader", "events", "pinned", "gameServers"]
        );
        let InfoviewSection::CommunityHeader {
            anchor,
            online_count,
            ..
        } = &view.sections[0]
        else {
            panic!("{view:?}");
        };
        assert_eq!(anchor, "c1");
        let my_row = host
            .governance()
            .community_members("c1")
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.pseudonym_key == me.to_hex())
            .expect("we are listed")
            .status;
        (*online_count, my_row)
    };

    // Only us, online: the count holds us, as our row does.
    host.ctx
        .own_presence
        .set_status(SessionStatus::Online)
        .await;
    assert_eq!(read().await, (1, "online".to_owned()), "only us online");

    // Invisible: our row shows offline and the count leaves us out.
    host.ctx
        .own_presence
        .set_status(SessionStatus::Invisible)
        .await;
    assert_eq!(read().await, (0, "offline".to_owned()), "us invisible");

    // Online again, and another member's beat shows present: one more.
    host.ctx
        .own_presence
        .set_status(SessionStatus::Online)
        .await;
    let now = host.ctx.clock.now_ms();
    let beat = |seq, state| PresenceBeat {
        seq,
        state,
        voice_channel: None,
        beat_number: now / rekindle_codec::member_record::beat::BEAT_PERIOD_MS,
        extras: None,
    };
    for (member, state) in [(1, BeatState::Online), (2, BeatState::Offline)] {
        crate::host::adapters::presence::view::observe(
            &host.ctx,
            "c1",
            &rekindle_types::id::PseudonymKey([member; 32]),
            &beat(1, state),
            Some(now),
        );
    }
    assert_eq!(
        read().await,
        (2, "online".to_owned()),
        "a member's beat adds one"
    );
}

/// `InfoviewGet { Selection }` for `window`, answered as JSON: its
/// `subject` is never `selection` (plan E7.15, IV5).
async fn get_selection(host: &Host, window: &str) -> InfoviewView {
    let request = IpcRequest::InfoviewGet {
        subject: InfoviewSubject::Selection,
        window: window.to_owned(),
    };
    let IpcResponse::Ok { data, .. } =
        rekindle_ipc::HostApi::call(host, Caller::shell(), request).await
    else {
        panic!("InfoviewGet refused");
    };
    assert_ne!(data["subject"]["type"], "selection", "{data}");
    serde_json::from_value(data).unwrap()
}

fn select(window: &str, selection: Option<SkinSelection>) -> IpcRequest {
    IpcRequest::SkinSelectionSet {
        window: window.to_owned(),
        selection,
    }
}

/// The Info popup asks `Selection`: it gets its launcher's selected
/// friend's page, labelled with the friend, never `Selection`.
#[tokio::test]
async fn a_popup_asking_selection_gets_its_launchers_selected_friend() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    let friend = SkinSelection::Friend {
        public_key: bob.clone(),
    };
    host.skin_window_opened("buddy-list", None, Some(friend));
    host.skin_window_opened("popup-info", Some("buddy-list"), None);
    let view = get_selection(&host, "popup-info").await;
    assert_eq!(view.subject, InfoviewSubject::Friend { public_key: bob });
    // Bob set no bio, so his page has no `bio` section.
    assert_eq!(kinds(&view), ["friendHeader", "nowPlaying"]);
}

/// Nothing selected in the launcher: our own page; a friend group selected:
/// that group's page.
#[tokio::test]
async fn a_popup_follows_no_selection_to_our_own_page_and_a_group_to_its_page() {
    let (_dirs, host, _owner, _bob) = ann_and_bob().await;
    host.skin_window_opened("buddy-list", None, None);
    host.skin_window_opened("popup-info", Some("buddy-list"), None);
    let view = get_selection(&host, "popup-info").await;
    assert_eq!(view.subject, InfoviewSubject::Own);
    assert_eq!(kinds(&view), ["activity", "communities", "nowPlaying"]);
    let group = SkinSelection::FriendGroup { group_id: None };
    host.skin_window_opened("buddy-list", None, Some(group));
    let view = get_selection(&host, "popup-info").await;
    assert_eq!(
        view.subject,
        InfoviewSubject::FriendGroup { group_id: None }
    );
    assert_eq!(kinds(&view), ["groupMembers"]);
}

/// A launcher's selection change pushes one `Updated`, with the resolved
/// subject, to the popup that follows it, and none to a popup whose
/// launcher did not change or to a window asking a fixed subject.
#[tokio::test]
async fn a_selection_change_pushes_only_the_popup_that_follows_it() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    host.skin_window_opened("buddy-list", None, None);
    host.skin_window_opened("chat-1", None, None);
    host.skin_window_opened("popup-info", Some("buddy-list"), None);
    host.skin_window_opened("popup-chat", Some("chat-1"), None);
    get_selection(&host, "popup-info").await;
    get_selection(&host, "popup-chat").await;
    get(&host, InfoviewSubject::Own, "buddy-list").await;
    // The opens above are older than one coalescing period by now.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let mut stream = host.ctx.journal.subscribe(None);
    let friend = SkinSelection::Friend {
        public_key: bob.clone(),
    };
    let request = select("buddy-list", Some(friend));
    let answer = rekindle_ipc::HostApi::call(&*host, Caller::shell(), request).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    let mut pushes = Vec::new();
    let end = tokio::time::Instant::now() + Duration::from_millis(900);
    while let Ok(Some(frame)) = tokio::time::timeout_at(end, stream.next()).await {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::System(SystemEvent::Infoview(InfoviewEvent::Updated {
                window,
                view,
            })) = s.event
            {
                pushes.push((window, view.subject));
            }
        }
    }
    assert_eq!(
        pushes,
        [(
            "popup-info".to_owned(),
            InfoviewSubject::Friend { public_key: bob }
        )]
    );
}
