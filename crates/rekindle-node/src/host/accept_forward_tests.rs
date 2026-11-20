//! Live finding L-FR2 (c): a friend's applied `FriendAccept` brings our
//! rows to them that wait out a backoff forward (`dm::route::reachable_again`
//! from `on_friend_accept`), so what we wrote before their accept goes at
//! once, not at the row's next due time.

use std::time::Duration;

use rekindle_crypto::Identity;
use rekindle_transport::operations::direct::replay_unprocessed;
use rekindle_types::message::SendState;

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, pending};
use super::game_totals_tests::accept;
use super::receive_tests::accepted;
use super::Host;

const ROW: &str = "dm-waiting";

/// Our `dm` row to `bob`, `queued` after one attempt and next due in a
/// day: a backoff no event has ended yet.
async fn waiting_row(host: &Host, owner: [u8; 32], bob: [u8; 32], due: i64) {
    let (o, stream) = (hex::encode(owner), format!("dm:{}", hex::encode(bob)));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            let row = rekindle_db::repo::outbox::NewRow {
                owner_key: &o,
                stream: &stream,
                client_id: ROW,
                kind: "dm",
                payload: b"frame",
                next_at: due,
                deadline: i64::MAX,
            };
            rekindle_db::repo::outbox::enqueue_with_echo(conn, &row, None)?;
            let backing_off = rekindle_db::repo::outbox::Update {
                state: SendState::Queued,
                attempts: 1,
                next_at: due,
                deadline: i64::MAX,
                wedge_reason: None,
            };
            rekindle_db::repo::outbox::update(conn, &o, ROW, &backing_off).map(drop)
        })
        .await
        .unwrap();
}

async fn row(host: &Host, owner: [u8; 32]) -> rekindle_db::repo::outbox::Row {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::outbox::get(conn, &o, ROW))
        .await
        .unwrap()
        .expect("the row is held")
}

/// An offline Host holds a backing-off `dm` row to Bob; Bob's
/// `FriendAccept` is applied and the row is due before its backoff ends
/// (attempted again at most one capped step later, never a day later).
#[tokio::test]
async fn an_applied_accept_brings_the_backing_off_rows_forward() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = Identity::generate().public_key_bytes();
    accepted(&host, owner, bob).await;
    let due = host.ctx.clock.now_ms_i64() + 86_400_000;
    waiting_row(&host, owner, bob, due).await;

    pending(&host, owner, bob, 1, &accept()).await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;

    let mut moved = None;
    for _ in 0..200 {
        let now = row(&host, owner).await;
        if now.next_at < due {
            moved = Some(now);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let moved = moved.expect("the accept brought the waiting row forward");
    assert!(moved.attempts >= 1, "its attempts are kept: {moved:?}");
}
