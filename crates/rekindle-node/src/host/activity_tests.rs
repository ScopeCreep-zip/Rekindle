//! The activity feed through the Host (plan E7.15, IV2): each friend change
//! kind the ingest stores, the presence hook, purge on unfriend and block,
//! and the "Keep an activity log" setting.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::Caller;
use rekindle_presence::{FriendPresenceDeps, GameInfoSnapshot};
use rekindle_types::activity::ActivityListItem;
use rekindle_types::friend::{FriendState, GameInfoState};
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::activity::{seen_of, store};
use super::adapters::friends::HostFriends;
use super::direct_tests::ann;
use super::game_totals_tests::{open_session, run};
use super::Host;

async fn feed(host: &Host) -> Vec<ActivityListItem> {
    let request = IpcRequest::ActivityList {
        before: None,
        limit: 100,
    };
    match rekindle_ipc::HostApi::call(host, Caller::shell(), request).await {
        IpcResponse::Ok { data, .. } => serde_json::from_value(data).unwrap(),
        other => panic!("{other:?}"),
    }
}

fn names(items: &[ActivityListItem]) -> Vec<&'static str> {
    items.iter().map(|i| i.kind.name()).collect()
}

fn friend_of(host: &Host, key: &str) -> FriendState {
    host.ctx
        .session
        .read()
        .as_ref()
        .and_then(|s| s.friends.get(key).cloned())
        .unwrap()
}

fn game(id: u32) -> GameInfoState {
    GameInfoState {
        game_id: id,
        game_name: format!("Game {id}"),
        server_info: None,
        elapsed_seconds: 30,
        icon_key: None,
        server_address: None,
    }
}

async fn change(host: &Host, prev: FriendState, next: FriendState) {
    let key = next.public_key.clone();
    store(
        &host.ctx,
        key,
        Some(seen_of(&prev, None)),
        seen_of(&next, None),
    )
    .await;
}

/// Two hosts with an open friendship; `host`'s view of `bob`.
async fn pair() -> (
    Vec<tempfile::TempDir>,
    std::sync::Arc<Host>,
    std::sync::Arc<Host>,
    String,
) {
    let dirs = vec![tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let (host, owner) = ann(&dirs[0]).await;
    let (bob_host, bob) = ann(&dirs[1]).await;
    open_session(&host, owner, &bob_host, bob).await;
    (dirs, host, bob_host, hex::encode(bob))
}

#[tokio::test]
async fn each_friend_change_makes_its_entry_with_the_hosts_names() {
    let (_dirs, host, _bob_host, bob) = pair().await;
    let base = friend_of(&host, &bob);
    let owner = host.ctx.require_owner().unwrap();
    let (o, b) = (owner.clone(), bob.clone());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| {
            rekindle_db::repo::friend_game_totals::replace(
                c,
                &o,
                &b,
                &[rekindle_db::repo::game_sessions::GameTotal {
                    game_id: 7,
                    total_secs: 60,
                    last_played: 1,
                }],
            )
        })
        .await
        .unwrap();

    let mut playing = base.clone();
    playing.game_info = Some(game(7));
    change(&host, base.clone(), playing.clone()).await;
    assert_eq!(
        names(&feed(&host).await),
        ["startedGame"],
        "a played game: no first time"
    );
    let mut new_game = base.clone();
    new_game.game_info = Some(game(9));
    change(&host, playing.clone(), new_game.clone()).await;
    assert_eq!(
        names(&feed(&host).await)[..3],
        ["firstPlayed", "startedGame", "stoppedGame"],
        "newest first: one change's rows by id, last stored first"
    );
    let mut renamed = base.clone();
    renamed.display_name = "Robert".into();
    renamed.status_message = Some("fragging".into());
    change(&host, base.clone(), renamed).await;
    store(
        &host.ctx,
        bob.clone(),
        Some(seen_of(&base, None)),
        seen_of(&base, Some("Quake".into())),
    )
    .await;
    let items = feed(&host).await;
    assert_eq!(
        names(&items)[..3],
        ["bioChanged", "statusMessage", "renamedSelf"]
    );
    assert_eq!(items.len(), 7);
    assert!(items
        .iter()
        .all(|i| i.display_name.as_deref() == Some(base.display_name.as_str())));

    // The presence hook: a watched game change reaches the feed.
    HostFriends::of(&host.ctx).set_friend_game_info(
        &bob,
        Some(GameInfoSnapshot {
            game_id: 7,
            game_name: "Game 7".into(),
            elapsed_seconds: 0,
            server_address: None,
        }),
    );
    for _ in 0..100 {
        if feed(&host).await.len() == 8 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(names(&feed(&host).await)[0], "startedGame");
}

#[tokio::test]
async fn unfriending_and_blocking_purge_their_entries() {
    let dirs = [(); 3].map(|()| tempfile::tempdir().unwrap());
    let (host, owner) = ann(&dirs[0]).await;
    let (bob_host, bob) = ann(&dirs[1]).await;
    let (carol_host, carol) = ann(&dirs[2]).await;
    open_session(&host, owner, &bob_host, bob).await;
    open_session(&host, owner, &carol_host, carol).await;
    for key in [hex::encode(bob), hex::encode(carol)] {
        let base = friend_of(&host, &key);
        let mut next = base.clone();
        next.game_info = Some(game(7));
        change(&host, base, next).await;
    }
    assert_eq!(feed(&host).await.len(), 2);
    // The removal's and the block's own transactions, without a network
    // (as `gate_tests` drives them).
    let db = host.ctx.db.current().unwrap();
    let (o, b) = (hex::encode(owner), hex::encode(bob));
    db.call(move |conn| {
        let stream = format!("friend_handshake:{b}");
        let row = rekindle_db::repo::outbox::NewRow {
            owner_key: &o,
            stream: &stream,
            client_id: "unfriended",
            kind: "friend_handshake",
            payload: b"{}",
            next_at: 2,
            deadline: 9,
            depends_on: None,
        };
        rekindle_db::repo::friends::mark_removing(conn, &o, &b, &row)
    })
    .await
    .unwrap();
    let left = feed(&host).await;
    assert_eq!(left.len(), 1, "removing them purges their entries");
    assert_eq!(left[0].kind.friend(), Some(hex::encode(carol).as_str()));
    let (o, c) = (hex::encode(owner), hex::encode(carol));
    db.call(move |conn| rekindle_db::repo::friends::block(conn, &o, &c, "Carol", 3))
        .await
        .unwrap();
    assert!(feed(&host).await.is_empty());
}

#[tokio::test]
async fn turning_the_log_off_clears_it_and_stores_nothing_until_on() {
    let (_dirs, host, _bob_host, bob) = pair().await;
    let base = friend_of(&host, &bob);
    let mut next = base.clone();
    next.game_info = Some(game(7));
    change(&host, base.clone(), next.clone()).await;
    assert_eq!(feed(&host).await.len(), 1);

    let (answer, _) = run(&host, IpcRequest::SetKeepActivityLog { enabled: false }).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    assert!(feed(&host).await.is_empty(), "off clears the feed");
    change(&host, next.clone(), base.clone()).await;
    assert!(feed(&host).await.is_empty(), "off stores nothing");

    let (answer, _) = run(&host, IpcRequest::SetKeepActivityLog { enabled: true }).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    change(&host, base, next).await;
    assert_eq!(names(&feed(&host).await), ["startedGame"]);
}

/// Store `bio` as `bob`'s and publish the `BioChanged` the transport
/// publishes for it.
async fn bio_changed(host: &Host, bob: &str, bio: &str) {
    let (o, b, text) = (
        host.ctx.require_owner().unwrap(),
        bob.to_owned(),
        bio.to_owned(),
    );
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::bio::set_friend(c, &o, &b, Some(&text)))
        .await
        .unwrap();
    host.ctx
        .publish_event(SubscriptionEvent::Friend(FriendEvent::BioChanged {
            peer_key: bob.to_owned(),
            bio_digest: blake3::hash(bio.as_bytes()).to_hex().to_string(),
        }));
}

async fn wait_for(host: &Host, len: usize) -> Vec<ActivityListItem> {
    for _ in 0..200 {
        let items = feed(host).await;
        if items.len() >= len {
            return items;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    feed(host).await
}

/// Two bio changes from one friend in a row are two entries: the journal's
/// dedup tells them apart by the new bio's digest.
#[tokio::test]
async fn two_bio_changes_in_a_row_are_two_entries() {
    let (_dirs, host, _bob_host, bob) = pair().await;
    // The worker holds Bob once it folded an event of his (as it holds a
    // friend from their `Accepted`); his game change is that event.
    HostFriends::of(&host.ctx).set_friend_game_info(
        &bob,
        Some(GameInfoSnapshot {
            game_id: 7,
            game_name: "Game 7".into(),
            elapsed_seconds: 0,
            server_address: None,
        }),
    );
    assert_eq!(names(&wait_for(&host, 1).await), ["startedGame"]);
    bio_changed(&host, &bob, "Quake").await;
    assert_eq!(names(&wait_for(&host, 2).await)[0], "bioChanged");
    bio_changed(&host, &bob, "Unreal").await;
    assert_eq!(
        names(&wait_for(&host, 3).await),
        ["bioChanged", "bioChanged", "startedGame"],
        "the second change is not dropped as a duplicate"
    );
}

/// A bio change as a friend's first event after an unlock makes an entry:
/// the worker compares it with what it last saw of them (its own stored
/// state), not the friend row the change already updated.
#[tokio::test]
async fn a_bio_change_first_after_unlock_makes_an_entry() {
    let (_dirs, host, _bob_host, bob) = pair().await;
    HostFriends::of(&host.ctx).set_friend_game_info(
        &bob,
        Some(GameInfoSnapshot {
            game_id: 7,
            game_name: "Game 7".into(),
            elapsed_seconds: 0,
            server_address: None,
        }),
    );
    assert_eq!(names(&wait_for(&host, 1).await), ["startedGame"]);
    // A new session: the worker drops what it holds in memory.
    host.ctx.journal.begin();
    bio_changed(&host, &bob, "Quake").await;
    assert_eq!(
        names(&wait_for(&host, 2).await),
        ["bioChanged", "startedGame"],
        "the bio is compared with the one the worker last saw"
    );
}
