//! A friend nickname through the Host's gate (plan E7.15 S3 item 2):
//! `FriendSetNickname` is Routine, so it runs unreviewed, stores the
//! nickname, announces it, and offers an undo that puts the old one back.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::friend::{FriendState, FriendshipState};

use super::direct_tests::ann;

#[tokio::test]
async fn a_friend_nickname_is_stored_announced_and_undone() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let (me, bob) = (hex::encode(owner), "b".repeat(64));
    let (o, p) = (me.clone(), bob.clone());
    let db = host.ctx.db.current().unwrap();
    db.call(move |conn| {
        rekindle_db::repo::friends::insert_accepted(conn, &o, &p, "Bob", 1)?;
        rekindle_db::repo::friends::set_nickname(conn, &o, &p, Some("Mac buddy"))
    })
    .await
    .unwrap();
    let mut state = FriendState::new(bob.clone(), "Bob".into(), FriendshipState::Accepted);
    state.nickname = Some("Mac buddy".into());
    host.ctx
        .session
        .write()
        .as_mut()
        .unwrap()
        .friends
        .insert(bob.clone(), state);
    let mut events = host.ctx.journal.subscribe(None);
    let stored = || {
        let (o, p) = (me.clone(), bob.clone());
        let db = db.clone();
        async move {
            db.call(move |conn| rekindle_db::repo::friends::names(conn, &o, &p))
                .await
                .unwrap()
                .and_then(|names| names.0)
        }
    };

    let set = IpcRequest::FriendSetNickname {
        public_key: bob.clone(),
        nickname: Some("Raid lead".into()),
    };
    let rekindle_ipc::protocol::IpcResponse::Ok { undo, .. } =
        host.call(Caller::shell(), set).await
    else {
        panic!("a Routine nickname runs unreviewed");
    };
    assert_eq!(stored().await.as_deref(), Some("Raid lead"));
    let mut seen = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        seen.push(format!("{frame:?}"));
    }
    assert!(
        seen.iter()
            .any(|f| f.contains("NicknameChanged") && f.contains("Raid lead")),
        "{seen:?}"
    );

    let offer = undo.expect("a nickname edit offers an undo");
    let undone = host
        .call(Caller::shell(), IpcRequest::Undo { id: offer.id })
        .await;
    assert!(
        matches!(undone, rekindle_ipc::protocol::IpcResponse::Ok { .. }),
        "{undone:?}"
    );
    assert_eq!(
        stored().await.as_deref(),
        Some("Mac buddy"),
        "the old nickname is back"
    );
}
