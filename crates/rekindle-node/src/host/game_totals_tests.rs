//! Game totals shared with friends through the Host (plan E7.2): nothing
//! leaves while sharing is off (on accept, after a session); turning it on
//! sends our totals to every accepted friend, whose copy replaces what
//! they held and is read back as `FriendGameTotalsGet`.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, FriendAccept};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_transport::operations::direct::{receive, replay_unprocessed};
use rekindle_transport::operations::dm::frame;
use rekindle_transport::operations::dm::handshake::client_prefix;
use rekindle_transport::operations::friend::game_totals::send_to_friends;
use rekindle_types::intent::{IntentClass, IntentProof};

use super::adapters::friends::HostFriends;
use super::direct_tests::{ann, pending, text};
use super::prekey_tests::own_bundle;
use super::receive_tests::accepted;
use super::Host;

/// `request` through the shell's gate, its review acknowledged; the
/// answer and the review's class.
pub(super) async fn run(host: &Host, request: IpcRequest) -> (IpcResponse, Option<IntentClass>) {
    match host.call(Caller::shell(), request).await {
        IpcResponse::IntentRequired(review) => {
            let commit = IpcRequest::IntentCommit {
                intent_id: review.intent_id,
                proof: IntentProof::Acknowledged,
            };
            (host.call(Caller::shell(), commit).await, Some(review.class))
        }
        answer => (answer, None),
    }
}

/// `peer` and `owner` each the other's accepted friend, with the session
/// `peer` opened by a first message to `owner`.
pub(super) async fn open_session(host: &Host, owner: [u8; 32], other: &Host, peer: [u8; 32]) {
    accepted(host, owner, peer).await;
    accepted(other, peer, owner).await;
    let bundle = own_bundle(host, owner).await;
    let sent = frame::initiate_frame(
        &HostFriends::of(&other.ctx),
        &hex::encode(owner),
        bundle,
        &text(1),
        frame::Position::default(),
    )
    .await
    .unwrap();
    receive(
        &HostFriends::of(&host.ctx),
        &rekindle_transport::operations::dm::wire(&sent).unwrap(),
    )
    .await;
}

/// Counter-Strike 2 played for a minute by `owner`.
async fn played(host: &Host, owner: [u8; 32]) {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::game_sessions::record(c, &o, 4181, 1_000, 61_000))
        .await
        .unwrap();
}

async fn set_sharing(host: &Host, owner: [u8; 32], on: bool) {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::preferences::set_share_game_totals(c, &o, on))
        .await
        .unwrap();
}

/// `owner`'s `GameTotals` rows, by stream.
async fn totals_rows(host: &Host, owner: [u8; 32]) -> Vec<rekindle_db::repo::outbox::Row> {
    let o = hex::encode(owner);
    let prefix = client_prefix(DmTypeId::GameTotals);
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

pub(super) fn accept() -> DirectBody {
    DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: String::new(),
        route_blob: Vec::new(),
        mailbox_dht_key: String::new(),
    })
}

/// The opt-in gate: with sharing off, neither Bob's accept nor a session's
/// send puts a `GameTotals` row on the outbox; on, both do.
#[tokio::test]
async fn nothing_is_sent_while_sharing_is_off() {
    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner) = ann(&dir).await;
    let (other, bob) = ann(&theirs).await;
    open_session(&host, owner, &other, bob).await;
    played(&host, owner).await;
    let friends = HostFriends::of(&host.ctx);

    pending(&host, owner, bob, 1, &accept()).await;
    replay_unprocessed(&friends).await;
    assert_eq!(send_to_friends(&friends).await, 0);
    assert!(
        totals_rows(&host, owner).await.is_empty(),
        "off by default: nothing leaves"
    );

    set_sharing(&host, owner, true).await;
    pending(&host, owner, bob, 2, &accept()).await;
    replay_unprocessed(&friends).await;
    assert_eq!(totals_rows(&host, owner).await.len(), 1, "sent on accept");
    assert_eq!(send_to_friends(&friends).await, 1);
    assert_eq!(
        totals_rows(&host, owner).await.len(),
        2,
        "sent after a session"
    );
}

/// The datagram a handshake row carries (`Framed::Wire`).
pub(super) fn datagram(row: &rekindle_db::repo::outbox::Row) -> Vec<u8> {
    #[derive(serde::Deserialize)]
    struct Wire {
        #[serde(with = "rekindle_types::base64_bytes")]
        datagram: Vec<u8>,
    }
    serde_json::from_slice::<Wire>(&row.payload)
        .unwrap()
        .datagram
}

/// Turning sharing on (TrustChanging) sends our totals to every accepted
/// friend; Bob's copy replaces what he held and reads back as Ann's.
#[tokio::test]
async fn turning_sharing_on_sends_to_every_friend_who_reads_them_back() {
    let dirs = [(); 3].map(|()| tempfile::tempdir().unwrap());
    let (host, owner) = ann(&dirs[0]).await;
    let (bob_host, bob) = ann(&dirs[1]).await;
    let (cy_host, cy) = ann(&dirs[2]).await;
    open_session(&host, owner, &bob_host, bob).await;
    open_session(&host, owner, &cy_host, cy).await;
    played(&host, owner).await;

    let (answer, class) = run(&host, IpcRequest::SetShareGameTotals { enabled: true }).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    assert_eq!(class, Some(IntentClass::TrustChanging));
    let rows = totals_rows(&host, owner).await;
    let mut streams: Vec<&str> = rows.iter().map(|r| r.stream.as_str()).collect();
    streams.sort_unstable();
    let mut expected = [
        format!("friend_handshake:{}", hex::encode(bob)),
        format!("friend_handshake:{}", hex::encode(cy)),
    ];
    expected.sort_unstable();
    assert_eq!(streams, expected, "one to each accepted friend");

    let to_bob = rows
        .iter()
        .find(|r| r.stream.ends_with(&hex::encode(bob)))
        .unwrap();
    receive(&HostFriends::of(&bob_host.ctx), &datagram(to_bob)).await;
    let read = bob_host
        .call(
            Caller::shell(),
            IpcRequest::FriendGameTotalsGet {
                public_key: hex::encode(owner),
            },
        )
        .await;
    let IpcResponse::Ok { data, .. } = read else {
        panic!("{read:?}");
    };
    assert_eq!(data[0]["gameId"], 4181);
    assert_eq!(data[0]["gameName"], "Counter-Strike 2");
    assert_eq!(data[0]["totalSecs"], 60);
    assert_eq!(data.as_array().unwrap().len(), 1);
}
