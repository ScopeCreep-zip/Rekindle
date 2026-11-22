//! The Infoview through the Host (plan E7.15, IV4): each subject composes
//! its sections from Host state, a shown event pushes the open window's
//! view once (coalesced), and a friend's page never lists communities.

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::Caller;
use rekindle_types::dto::infoview::{
    InfoviewEvent, InfoviewSection, InfoviewSubject, InfoviewView,
};
use rekindle_types::friend::GameInfoState;
use rekindle_types::subscription_events::{
    FriendEvent, StreamFrame, SubscriptionEvent, SystemEvent,
};

use super::activity::{seen_of, store};
use super::direct_tests::ann;
use super::game_totals_tests::open_session;
use super::seed_registry_community;
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
async fn ann_and_bob() -> (Vec<tempfile::TempDir>, std::sync::Arc<Host>, String, String) {
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
        serde_json::to_value(rows[0].join.as_ref().unwrap()).unwrap(),
        serde_json::json!({"type": "joinServer", "gameId": 4100, "server": "203.0.113.7:27015"})
    );
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
    assert!(!json.contains("communit"), "{json}");
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

/// Two shown events in one period push the open window's view once.
#[tokio::test]
async fn shown_events_push_the_open_window_once() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    let mut stream = host.ctx.journal.subscribe(None);
    get(&host, InfoviewSubject::Own, "main").await;
    for nick in ["A", "B"] {
        host.ctx
            .publish_event(SubscriptionEvent::Friend(FriendEvent::NicknameChanged {
                peer_key: bob.clone(),
                nickname: Some(nick.into()),
            }));
    }
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
    assert_eq!(pushes, [("main".to_owned(), InfoviewSubject::Own)]);
}

/// A community's page: its header with the online count its beats show,
/// then its events, pins and game servers.
#[tokio::test]
async fn a_community_page_counts_the_members_its_beats_show_present() {
    use rekindle_codec::member_record::{BeatState, PresenceBeat};
    let (_dirs, host, owner, _bob) = ann_and_bob().await;
    seed_registry_community(&host, &owner).await;
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
    assert_eq!((anchor.as_str(), *online_count), ("c1", 1));
}
