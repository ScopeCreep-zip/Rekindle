//! The activity feed through the friend ingest (plan E7.15, IV2; Infoview
//! design §2.3): a friend starting a game makes exactly one `StartedGame`
//! on our side; a game absent from the totals they shared before is also a
//! `FirstPlayed`, and one already in them is not. Consent follows the
//! sender (design §3.3): a friend not sharing their totals gives no
//! `FirstPlayed`, only the game they show.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::activity::{ActivityKind, ActivityListItem};

use rekindle_netsim::Netsim;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, LIMIT};

/// A game Rae played before (in the totals she shares) and one she has not.
const PLAYED: u32 = 3;
const NEW: u32 = 7;
/// How long the feed is watched after the first entry for a duplicate.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(60);

async fn feed(node: &SimNode) -> Vec<ActivityListItem> {
    let page = node
        .ok(IpcRequest::ActivityList {
            before: None,
            limit: 100,
        })
        .await;
    serde_json::from_value(page).expect("an activity page")
}

fn started(items: &[ActivityListItem], friend: &str, game: u32) -> usize {
    items
        .iter()
        .filter(|i| matches!(&i.kind, ActivityKind::StartedGame { friend: f, game_id, .. } if f == friend && *game_id == game))
        .count()
}

fn first_played(items: &[ActivityListItem], friend: &str, game: u32) -> usize {
    items
        .iter()
        .filter(|i| matches!(&i.kind, ActivityKind::FirstPlayed { friend: f, game_id } if f == friend && *game_id == game))
        .count()
}

fn any_first_played(items: &[ActivityListItem]) -> bool {
    items
        .iter()
        .any(|i| matches!(i.kind, ActivityKind::FirstPlayed { .. }))
}

async fn play(node: &SimNode, game: u32) {
    node.ok(IpcRequest::GamePresenceSet {
        game_name: format!("game {game}"),
        game_id: Some(game),
        elapsed_seconds: 0,
        server_address: None,
    })
    .await;
}

/// Wait until `node`'s feed holds a `StartedGame` of `game` by `friend`,
/// then run the sim on for `SETTLE` so a duplicate would have landed too.
async fn feed_after_start(
    sim: &Netsim,
    node: &SimNode,
    friend: &str,
    game: u32,
) -> Vec<ActivityListItem> {
    assert!(
        eventually(sim, LIMIT, || async {
            started(&feed(node).await, friend, game) >= 1
        })
        .await,
        "the feed shows {friend} starting game {game}"
    );
    let _ = eventually(sim, SETTLE, || async { false }).await;
    feed(node).await
}

/// Rae's one recorded session of `PLAYED`, as her game publisher records
/// a detected session when it ends.
async fn seed_played(node: &SimNode) {
    let db = node
        .host
        .identity_stores()
        .db
        .current()
        .expect("an open database");
    let owner = key(node);
    db.call(move |conn| {
        rekindle_db::repo::game_sessions::record(
            conn,
            &owner,
            PLAYED,
            1_700_000_000_000,
            1_700_000_600_000,
        )
    })
    .await
    .expect("the session is recorded");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_started_game_is_one_entry_and_a_first_time_is_also_first_played() {
    let (sim, mut nodes) = start(2, 93).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let rae = key(&nodes[0]);
    seed_played(&nodes[0]).await;
    nodes[0]
        .ok(IpcRequest::SetShareGameTotals { enabled: true })
        .await;
    let totals_arrived = || async {
        let totals = nodes[1]
            .ok(IpcRequest::FriendGameTotalsGet {
                public_key: rae.clone(),
            })
            .await;
        totals.as_array().is_some_and(|t| !t.is_empty())
    };
    assert!(
        eventually(&sim, LIMIT, totals_arrived).await,
        "Ann holds the totals Rae shares"
    );

    play(&nodes[0], NEW).await;
    let items = feed_after_start(&sim, &nodes[1], &rae, NEW).await;
    assert_eq!(started(&items, &rae, NEW), 1, "one StartedGame: {items:?}");
    assert_eq!(
        first_played(&items, &rae, NEW),
        1,
        "a game not in Rae's totals is a first time: {items:?}"
    );

    nodes[0].ok(IpcRequest::GamePresenceClear).await;
    play(&nodes[0], PLAYED).await;
    let items = feed_after_start(&sim, &nodes[1], &rae, PLAYED).await;
    assert_eq!(
        started(&items, &rae, PLAYED),
        1,
        "one StartedGame: {items:?}"
    );
    assert_eq!(
        first_played(&items, &rae, PLAYED),
        0,
        "a game in Rae's totals is not a first time: {items:?}"
    );
    assert_eq!(started(&items, &rae, NEW), 1, "{items:?}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn without_shared_totals_a_started_game_is_never_a_first_time() {
    let (sim, mut nodes) = start(2, 94).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let rae = key(&nodes[0]);
    seed_played(&nodes[0]).await;

    play(&nodes[0], NEW).await;
    let items = feed_after_start(&sim, &nodes[1], &rae, NEW).await;
    assert_eq!(started(&items, &rae, NEW), 1, "one StartedGame: {items:?}");
    assert!(
        !any_first_played(&items),
        "Rae shares no totals, so nothing is a first time: {items:?}"
    );
}
