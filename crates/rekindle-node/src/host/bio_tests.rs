//! Our bio through the Host (plan E7.2): `ProfileSetBio` stores it and
//! sends a ratchet-sealed `ProfileBio` to every accepted friend, whose copy
//! reads back as `FriendBioGet`; blank removes it on both ends; past 190
//! characters is refused.

use rekindle_codec::capnp_codec::direct::DmTypeId;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_transport::operations::direct::receive;
use rekindle_transport::operations::dm::handshake::client_prefix;

use super::adapters::friends::HostFriends;
use super::direct_tests::ann;
use super::game_totals_tests::{datagram, open_session, run};
use super::Host;

/// `owner`'s `ProfileBio` rows, oldest first.
async fn bio_rows(host: &Host, owner: [u8; 32]) -> Vec<rekindle_db::repo::outbox::Row> {
    let o = hex::encode(owner);
    let prefix = client_prefix(DmTypeId::ProfileBio);
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

async fn read(host: &Host, request: IpcRequest) -> serde_json::Value {
    match host.call(Caller::shell(), request).await {
        IpcResponse::Ok { data, .. } => data,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_bio_reaches_each_friend_and_blank_clears_it() {
    let dirs = [(); 2].map(|()| tempfile::tempdir().unwrap());
    let (host, owner) = ann(&dirs[0]).await;
    let (bob_host, bob) = ann(&dirs[1]).await;
    open_session(&host, owner, &bob_host, bob).await;
    let ann_key = || IpcRequest::FriendBioGet {
        public_key: hex::encode(owner),
    };

    let (answer, class) = run(
        &host,
        IpcRequest::ProfileSetBio {
            bio: "x".repeat(191),
        },
    )
    .await;
    assert!(
        matches!(answer, IpcResponse::Error { code: 400, .. }),
        "{answer:?}"
    );
    assert_eq!(class, None, "refused before any review");

    let (answer, class) = run(
        &host,
        IpcRequest::ProfileSetBio {
            bio: "Quake 3 since '99".into(),
        },
    )
    .await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    assert_eq!(class, None, "Routine: committed without a review");
    assert_eq!(
        read(&host, IpcRequest::ProfileBioGet).await,
        "Quake 3 since '99"
    );
    let rows = bio_rows(&host, owner).await;
    assert_eq!(rows.len(), 1, "one to the accepted friend");
    assert!(rows[0].stream.ends_with(&hex::encode(bob)));
    assert_eq!(read(&bob_host, ann_key()).await, serde_json::Value::Null);
    receive(&HostFriends::of(&bob_host.ctx), &datagram(&rows[0])).await;
    assert_eq!(read(&bob_host, ann_key()).await, "Quake 3 since '99");

    let (answer, _) = run(&host, IpcRequest::ProfileSetBio { bio: " ".into() }).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    assert_eq!(
        read(&host, IpcRequest::ProfileBioGet).await,
        serde_json::Value::Null
    );
    let rows = bio_rows(&host, owner).await;
    assert_eq!(rows.len(), 2, "the removal is sent too");
    receive(&HostFriends::of(&bob_host.ctx), &datagram(&rows[1])).await;
    assert_eq!(read(&bob_host, ann_key()).await, serde_json::Value::Null);
}
