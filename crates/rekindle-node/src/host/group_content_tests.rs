//! Group content on the Host (plan E2.6 G2c) over a real unlocked identity:
//! our send is one `group_dm` row with its echo and is not sent until its
//! chain entry lands; a member's chain entries open and store in order
//! under the group's conversation, an older epoch is refused and a newer
//! one waits; typing is a hint only; a text hint stores at once and its
//! chain copy is a duplicate (finding G3); a retired stretch is a history
//! gap only when one of its entries is not held by hint (G4).

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
    // The attempt hands the sealed entry to the stream's appender (the DM
    // pump); offline, no chain entry lands, so the row is not sent.
    let row = RowAttempt {
        stream_seq: rows[0].stream_seq,
        state: rows[0].state,
        sent_at: rows[0].sent_at,
        payload: &rows[0].payload,
    };
    let tried = content::attempt(&friends, &gid, &[row]).await;
    let [Attempt::Appending(frame)] = tried.as_slice() else {
        panic!("handed off: {tried:?}");
    };
    let landed = content::append_frames(&friends, &gid, &[(frame.clone(), row.stream_seq)]).await;
    assert_eq!(landed, [Attempt::Deferred]);
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

/// Finding G3: the hint is sent before the chain append lands, so a hint
/// that only diffed applied nothing until the chain's watch fired. Its
/// content stores on receipt; the chain's copy later passes as a duplicate
/// (no second row or event) and moves the cursor.
#[tokio::test]
async fn a_text_hint_stores_at_once_and_its_chain_copy_is_a_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let key = group(&friends, owner).await;
    let me = hex::encode(owner);
    let mut events = host.ctx.journal.subscribe(None);
    // No chain of Bob's is held: nothing but the hint can store it.
    let hint = sealed(&key, &text(7, "now"), 7);
    assert!(receive::on_hint(&friends, &bob(), &hint).await);
    assert_eq!(
        conversation(&friends, &me, 7).await,
        Some((hex::encode(GID), ConversationType::GroupDm)),
        "the hint's content is stored on receipt"
    );
    let (sink, stream) = (
        GroupSink::new(&friends, GID),
        stream_name(&hex::encode(GID)),
    );
    for n in 1..=7 {
        sink.apply(&stream, &bob(), &entry(&key, n)).await.unwrap();
    }
    assert_eq!(sink.cursor(&stream, &bob()).await.unwrap(), 7);
    let mut stored = 0;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        let f = format!("{frame:?}");
        if f.contains("Message {")
            && f.contains(&rekindle_transport::operations::dm::client_id_text(
                &[7; 16],
            ))
        {
            stored += 1;
        }
    }
    assert_eq!(stored, 1, "the chain copy is a duplicate");
}

/// G4: Bob's hints `held` are stored (each by its outbox stream seq, the
/// cursor unmoved), then his chain retires entries 1–3 before our diff
/// reads it, handing the sink entry 4 (`next`) or none. Whether a gap was
/// shown, and the cursor after.
async fn retired_stretch(held: &[u8], next: bool) -> (bool, u64) {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let key = group(&friends, owner).await;
    let mut events = host.ctx.journal.subscribe(None);
    for &n in held {
        let hint = sealed(&key, &text(n, "hint"), u64::from(n));
        assert!(receive::on_hint(&friends, &bob(), &hint).await);
    }
    let (sink, stream) = (
        GroupSink::new(&friends, GID),
        stream_name(&hex::encode(GID)),
    );
    assert_eq!(
        sink.cursor(&stream, &bob()).await.unwrap(),
        0,
        "hints never move it"
    );
    let after = next.then(|| entry(&key, 4));
    sink.history_gap(&stream, &bob(), 3, after.as_ref())
        .await
        .unwrap();
    let mut shown = false;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        shown |= format!("{frame:?}").contains("HistoryGap");
    }
    (shown, sink.cursor(&stream, &bob()).await.unwrap())
}

#[tokio::test]
async fn a_retired_stretch_held_by_hints_is_no_gap() {
    assert_eq!(retired_stretch(&[1, 2, 3], true).await, (false, 3));
}

#[tokio::test]
async fn a_retired_stretch_missing_a_hint_is_a_gap() {
    assert_eq!(retired_stretch(&[1, 3], true).await, (true, 3));
}

#[tokio::test]
async fn a_retired_stretch_with_no_entry_after_the_mark_is_a_gap() {
    assert_eq!(retired_stretch(&[1, 2, 3], false).await, (true, 3));
}

/// G4: an applied chain entry records its outbox stream seq at the
/// cursor, and a later stretch is judged from it, not from the stream's
/// start: Bob's chain was made again (its seq restarts, the stream seq
/// does not), so his entries 1–2 carry stream seqs 5–6 and nothing of 1–4
/// is stored here; hint 7 is stored, then chain entry 3 retires before the
/// chain is read, and entry 4 carries stream seq 8.
#[tokio::test]
async fn a_stretch_after_applied_entries_is_judged_from_the_cursors_stream_seq() {
    fn entry_at(key: &GroupSenderKey, seq: u64, stream_seq: u8) -> ChainEntry {
        let content = sealed(key, &text(stream_seq, "hi"), u64::from(stream_seq));
        ChainEntry {
            seq,
            ciphertext: DirectBody::GroupContent(content).encode(),
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let key = group(&friends, owner).await;
    let (sink, stream) = (
        GroupSink::new(&friends, GID),
        stream_name(&hex::encode(GID)),
    );
    for (seq, stream_seq) in [(1, 5), (2, 6)] {
        sink.apply(&stream, &bob(), &entry_at(&key, seq, stream_seq))
            .await
            .unwrap();
    }
    let hint = sealed(&key, &text(7, "hint"), 7);
    assert!(receive::on_hint(&friends, &bob(), &hint).await);
    let mut events = host.ctx.journal.subscribe(None);
    sink.history_gap(&stream, &bob(), 3, Some(&entry_at(&key, 4, 8)))
        .await
        .unwrap();
    let mut shown = false;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        shown |= format!("{frame:?}").contains("HistoryGap");
    }
    assert!(!shown, "6 at the cursor, 7 held by hint");
    assert_eq!(sink.cursor(&stream, &bob()).await.unwrap(), 3);
}
