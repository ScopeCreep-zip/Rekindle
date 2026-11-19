//! Game invites through the Host (plan E7.2): sent only to an accepted
//! friend, never for a hidden game or while current-game sharing is off;
//! received only from an accepted friend, as `FriendEvent::GameInvite`.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, GameInvite};
use rekindle_crypto::Identity;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_transport::operations::direct::{receive, replay_unprocessed};
use rekindle_transport::operations::dm::handshake::client_prefix;
use rekindle_transport::operations::friend::FriendHost as _;
use rekindle_types::friend::FriendshipState;
use rekindle_types::game_server::ServerAddress;

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, pending};
use super::game_totals_tests::{datagram, open_session, run};
use super::receive_tests::{drain, mentions};
use super::Host;

const CS2: u32 = 4181;

fn invite(to: [u8; 32]) -> IpcRequest {
    IpcRequest::GameInviteSend {
        public_key: hex::encode(to),
        game_id: CS2,
        server: ServerAddress::parse("203.0.113.7:27015").unwrap(),
    }
}

async fn privacy(host: &Host, owner: [u8; 32], share: bool, hidden: bool) {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| {
            rekindle_db::repo::preferences::set_share_current_game(c, &o, share)?;
            if hidden {
                rekindle_db::repo::hidden_games::hide(c, &o, CS2)
            } else {
                rekindle_db::repo::hidden_games::unhide(c, &o, CS2)
            }
        })
        .await
        .unwrap();
}

/// `owner`'s `GameInvite` rows on the outbox.
async fn invite_rows(host: &Host, owner: [u8; 32]) -> Vec<rekindle_db::repo::outbox::Row> {
    let o = hex::encode(owner);
    let prefix = client_prefix(DmTypeId::GameInvite);
    let rows = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::outbox::prefix_rows(c, &o, ""))
        .await
        .unwrap();
    rows.into_iter()
        .filter(|row| row.client_id.starts_with(&prefix))
        .collect()
}

/// `host`'s friend `peer` moved out of `Accepted` (an unfriend under way),
/// its session kept.
fn removing(host: &Host, peer: [u8; 32]) {
    let friends = HostFriends::of(&host.ctx);
    let session = friends.session();
    let mut guard = session.write();
    let friend = guard
        .as_mut()
        .unwrap()
        .friends
        .get_mut(&hex::encode(peer))
        .unwrap();
    friend.friendship_state = FriendshipState::Removing;
}

fn refused(answer: &IpcResponse) -> bool {
    matches!(answer, IpcResponse::Error { .. })
}

/// An invite says what we play: none while current-game sharing is off,
/// none for a hidden game, none to anyone but an accepted friend. With
/// sharing on and the game shown, Bob receives it as a `GameInvite` event.
#[tokio::test]
async fn invites_go_only_to_friends_for_a_shared_shown_game() {
    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner) = ann(&dir).await;
    let (other, bob) = ann(&theirs).await;
    open_session(&host, owner, &other, bob).await;

    let (answer, _) = run(&host, invite(bob)).await;
    assert!(refused(&answer), "sharing is off by default: {answer:?}");
    privacy(&host, owner, true, true).await;
    let (answer, _) = run(&host, invite(bob)).await;
    assert!(
        refused(&answer),
        "a hidden game is never offered: {answer:?}"
    );
    privacy(&host, owner, true, false).await;
    let stranger = Identity::generate().public_key_bytes();
    let (answer, _) = run(&host, invite(stranger)).await;
    assert!(refused(&answer), "a stranger gets none: {answer:?}");
    assert!(invite_rows(&host, owner).await.is_empty(), "nothing left");

    let (answer, _) = run(&host, invite(bob)).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    let rows = invite_rows(&host, owner).await;
    assert_eq!(rows.len(), 1);
    let mut events = other.ctx.journal.subscribe(None);
    receive(&HostFriends::of(&other.ctx), &datagram(&rows[0])).await;
    let seen = drain(&mut events).await;
    assert_eq!(mentions(&seen, "GameInvite"), 1, "{seen:?}");
    assert_eq!(mentions(&seen, "203.0.113.7:27015"), 1, "{seen:?}");

    // A second invite reaches Bob after he began unfriending Ann: dropped.
    let (answer, _) = run(&host, invite(bob)).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    let rows = invite_rows(&host, owner).await;
    removing(&other, owner);
    receive(
        &HostFriends::of(&other.ctx),
        &datagram(&rows[rows.len() - 1]),
    )
    .await;
    assert_eq!(mentions(&drain(&mut events).await, "GameInvite"), 0);
    // And Ann, unfriending Bob with the session still open, sends none.
    removing(&host, bob);
    let (answer, _) = run(&host, invite(bob)).await;
    assert!(refused(&answer), "not an accepted friend: {answer:?}");
}

/// A `GameInvite` from anyone but an accepted friend is dropped, unseen.
#[tokio::test]
async fn an_invite_from_a_stranger_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let stranger = Identity::generate().public_key_bytes();
    let mut events = host.ctx.journal.subscribe(None);
    let body = DirectBody::GameInvite(GameInvite {
        game_id: CS2,
        server: ServerAddress::parse("203.0.113.7:27015").unwrap(),
    });
    pending(&host, owner, stranger, 1, &body).await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    assert_eq!(mentions(&drain(&mut events).await, "GameInvite"), 0);
}
