//! A friend code asked for before our records are held (plan E2.5 FC1):
//! the answer waits on the own-records stage itself and comes the moment
//! the stage holds our profile and mailbox, however long that took (Bob's
//! took ~42 s live, past the old fixed 30 s wait that answered `NotReady`);
//! a stage that ends without them answers its failure.

use std::time::Duration;

use rekindle_transport::operations::friend::{code, FriendError, OwnRecordsStage};

use super::{create_ann, offline_host};
use crate::host::adapters::friends::HostFriends;
use crate::host::Host;

const PROFILE: &str = "VLD0:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const MAILBOX: &str = "VLD0:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA";

type Asked =
    tokio::task::JoinHandle<Result<rekindle_types::dto::friends::FriendCodeView, FriendError>>;

/// Ann unlocked offline (her own-records stage never ends), asking for
/// her friend code, still unanswered 45 s later.
async fn asked_while_unheld(host: &Host) -> Asked {
    create_ann(host).await;
    let friends = HostFriends::of(&host.ctx);
    let asked = tokio::spawn(async move { code::own(&friends).await });
    // The request is under way (its wait started) before the clock moves.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(45)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(
        !asked.is_finished(),
        "the code answered before the records were held"
    );
    asked
}

#[tokio::test]
async fn a_friend_code_asked_before_the_records_are_held_answers_once_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let asked = asked_while_unheld(&host).await;

    // The stage's end: the projection names the records, then it is told.
    {
        let mut session = host.ctx.session.write();
        let identity = &mut session.as_mut().expect("Ann is unlocked").identity;
        identity.profile_dht_key = PROFILE.into();
        identity.mailbox_dht_key = MAILBOX.into();
    }
    host.ctx
        .friend_stores
        .own_records
        .send_replace(OwnRecordsStage::Held);
    tokio::time::resume();
    let code = tokio::time::timeout(Duration::from_secs(10), asked)
        .await
        .expect("answered once held")
        .unwrap()
        .expect("the friend code");
    assert_eq!(code.profile_key, PROFILE);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_stage_that_ends_without_the_records_answers_its_failure() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let asked = asked_while_unheld(&host).await;

    host.ctx
        .friend_stores
        .own_records
        .send_replace(OwnRecordsStage::Failed("no record pool".into()));
    tokio::time::resume();
    let refused = tokio::time::timeout(Duration::from_secs(10), asked)
        .await
        .expect("answered once the stage ended")
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(&refused, FriendError::Failed(why) if why.contains("no record pool")),
        "{refused}"
    );
    host.shutdown().await.unwrap();
}
