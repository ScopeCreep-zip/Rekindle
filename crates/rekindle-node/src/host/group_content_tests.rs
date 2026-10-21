//! Group content on the Host (plan E2.6 G2c) over a real unlocked identity:
//! our send is one `group_dm` row with its echo and is not sent until its
//! chain entry lands; a member's chain entries open and store in order
//! under the group's conversation, an older epoch is refused and a newer
//! one waits; typing is a hint only.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmText, GroupContent, GroupId, Typing};
use rekindle_dm::sender_key;
use rekindle_dm::{GroupDmDeps, GroupSenderKey};
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::StreamSink;
use rekindle_transport::operations::dm::outbox::{Attempt, RowAttempt};
use rekindle_transport::operations::group_dm::content::{self, inner, stream_name};
use rekindle_transport::operations::group_dm::receive::{self, GroupSink};
use rekindle_types::message::ConversationType;
use rekindle_utils::OsRandom;

use super::adapters::friends::HostFriends;
use super::adapters::group_dm::HostGroupDm;
use super::direct_tests::ann;
use super::group_dm_tests::member;

const GID: GroupId = [9; 16];

fn bob() -> String {
    "b".repeat(64)
}

/// Ann's group with Bob and Carol, Bob's first key held.
async fn group(host: &HostFriends, owner: [u8; 32]) -> GroupSenderKey {
    let deps = HostGroupDm::open(host).unwrap();
    let others = vec![member(bob(), 'b'), member("c".repeat(64), 'c')];
    let own = member(hex::encode(owner), 'a');
    let chain = format!("VLD0:{}", "Q".repeat(43));
    rekindle_dm::create(&deps, GID, "raid", own, others, &chain)
        .await
        .unwrap();
    let key = sender_key::mint_first(&OsRandom);
    deps.put_sender_key(&GID, &bob(), &key).await.unwrap();
    key
}

fn sealed(key: &GroupSenderKey, body: &DirectBody, seq: u64) -> GroupContent {
    sender_key::seal(key, GID, &bob(), &inner(body, 1_000 + seq, seq)).unwrap()
}

fn text(n: u8, body: &str) -> DirectBody {
    DirectBody::DmText(DmText {
        client_id: [n; 16],
        body: body.into(),
        reply_to: None,
        expire_seconds: 0,
    })
}

fn entry(key: &GroupSenderKey, n: u8) -> ChainEntry {
    let content = sealed(key, &text(n, "hi"), u64::from(n));
    ChainEntry {
        seq: u64::from(n),
        ciphertext: DirectBody::GroupContent(content).encode(),
    }
}

async fn conversation(
    host: &HostFriends,
    owner: &str,
    n: u8,
) -> Option<(String, ConversationType)> {
    let (owner, id) = (
        owner.to_owned(),
        rekindle_transport::operations::dm::client_id_text(&[n; 16]),
    );
    let db = host.ctx().db.current().unwrap();
    db.call(move |c| rekindle_db::repo::messages::conversation_of(c, &owner, &id))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_send_commits_its_row_and_echo_and_waits_for_its_chain_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    group(&friends, owner).await;
    let gid = hex::encode(GID);
    let id = content::send(&friends, &gid, "hello").await.unwrap();

    let (o, stream) = (hex::encode(owner), stream_name(&gid));
    let db = host.ctx.db.current().unwrap();
    let rows = db
        .call(move |c| rekindle_db::repo::outbox::stream_rows(c, &o, &stream))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].kind.as_str(), rows[0].client_id.as_str()),
        ("group_dm", id.as_str())
    );
    let (o, mid) = (hex::encode(owner), id.clone());
    let echo = db
        .call(move |c| rekindle_db::repo::messages::conversation_of(c, &o, &mid))
        .await
        .unwrap();
    assert_eq!(echo, Some((gid.clone(), ConversationType::GroupDm)));
    // Offline: no chain entry lands, so the row is not sent.
    let row = RowAttempt {
        stream_seq: rows[0].stream_seq,
        sent_at: rows[0].sent_at,
        payload: &rows[0].payload,
    };
    let tried = content::attempt(&friends, &gid, &[row]).await;
    assert_eq!(tried, [Attempt::Deferred]);
    assert!(
        content::send(&friends, &"0".repeat(32), "x").await.is_err(),
        "not our group"
    );
}

#[tokio::test]
async fn a_members_entries_store_in_order_and_a_wrong_epoch_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let first = group(&friends, owner).await;
    let (sink, stream, me) = (
        GroupSink::new(&friends, GID),
        stream_name(&hex::encode(GID)),
        hex::encode(owner),
    );
    sink.apply(&stream, &bob(), &entry(&first, 1))
        .await
        .unwrap();
    sink.apply(&stream, &bob(), &entry(&first, 2))
        .await
        .unwrap();
    assert_eq!(sink.cursor(&stream, &bob()).await.unwrap(), 2);
    for n in [1, 2] {
        assert_eq!(
            conversation(&friends, &me, n).await,
            Some((hex::encode(GID), ConversationType::GroupDm))
        );
    }
    // Bob rotated: an entry under his older key is refused and skipped.
    let second = sender_key::rotate(&first, &OsRandom).unwrap();
    let deps = HostGroupDm::open(&friends).unwrap();
    deps.put_sender_key(&GID, &bob(), &second).await.unwrap();
    sink.apply(&stream, &bob(), &entry(&first, 3))
        .await
        .unwrap();
    assert_eq!(conversation(&friends, &me, 3).await, None);
    assert_eq!(sink.cursor(&stream, &bob()).await.unwrap(), 3);
    // A newer epoch than held waits for its key: the cursor stays.
    let third = sender_key::rotate(&second, &OsRandom).unwrap();
    assert!(sink
        .apply(&stream, &bob(), &entry(&third, 4))
        .await
        .is_err());
    assert_eq!(sink.cursor(&stream, &bob()).await.unwrap(), 3);
}

#[tokio::test]
async fn typing_is_a_hint_and_never_stored() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let key = group(&friends, owner).await;
    let mut events = host.ctx.journal.subscribe(None);
    let typing = sealed(&key, &DirectBody::Typing(Typing { typing: true }), 0);
    assert!(receive::on_hint(&friends, &bob(), &typing).await);
    let stale = sender_key::rotate(&key, &OsRandom).unwrap();
    let deps = HostGroupDm::open(&friends).unwrap();
    deps.put_sender_key(&GID, &bob(), &stale).await.unwrap();
    assert!(
        !receive::on_hint(&friends, &bob(), &typing).await,
        "older epoch refused"
    );
    let mut seen = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        seen.push(format!("{frame:?}"));
    }
    assert!(
        seen.iter()
            .any(|f| f.contains("GroupDm") && f.contains("Started")),
        "{seen:?}"
    );
    assert!(
        !seen.iter().any(|f| f.contains("Message {")),
        "typing is never stored: {seen:?}"
    );
}
