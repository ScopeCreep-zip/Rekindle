//! Group DM replies (owed item GD): our reply's parent id rides inside the
//! sealed `DmText` like the body, is stored on our row and served by the
//! group's history and our `MessageSent`; a member's reply naming a
//! message we never received is stored and served as-is (the shell shows
//! "message unavailable"), never dropped; a reply whose seal was tampered
//! with or forged under a key we do not hold is refused like any body.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmText};
use rekindle_dm::sender_key;
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::StreamSink;
use rekindle_transport::operations::dm::client_id_text;
use rekindle_transport::operations::group_dm::content::{self, stream_name};
use rekindle_transport::operations::group_dm::messages;
use rekindle_transport::operations::group_dm::receive::GroupSink;
use rekindle_types::subscription_events::{GroupDmEvent, StreamFrame, SubscriptionEvent};
use rekindle_utils::OsRandom;

use super::adapters::friends::HostFriends;
use super::direct_tests::ann;
use super::group_content_tests::{bob, group, sealed, GID};

fn reply(n: u8, parent: [u8; 16]) -> DirectBody {
    DirectBody::DmText(DmText {
        client_id: [n; 16],
        body: format!("reply {n}"),
        reply_to: Some(parent),
        expire_seconds: 0,
    })
}

async fn reply_of(friends: &HostFriends, gid: &str, id: &str) -> Option<Option<String>> {
    messages(friends, gid, 50, None)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.message_id.as_deref() == Some(id))
        .map(|m| m.reply_to_id)
}

#[tokio::test]
async fn our_group_reply_is_stored_served_and_published_with_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    group(&friends, owner).await;
    let gid = hex::encode(GID);
    let parent = content::send(&friends, &gid, "first", None).await.unwrap();
    let mut events = host.ctx.journal.subscribe(None);

    let id = content::send(&friends, &gid, "re: first", Some(&parent))
        .await
        .unwrap();

    let mut sent = None;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        if let StreamFrame::Event(seq) = frame {
            if let SubscriptionEvent::GroupDm(GroupDmEvent::MessageSent {
                reply_to_id,
                message,
                ..
            }) = seq.event
            {
                sent = Some((reply_to_id, message));
            }
        }
    }
    let (reply_to_id, message) = sent.expect("our reply is published");
    assert_eq!(reply_to_id.as_deref(), Some(parent.as_str()));
    assert_eq!(message.reply_to_id.as_deref(), Some(parent.as_str()));
    assert_eq!(
        reply_of(&friends, &gid, &id).await,
        Some(Some(parent.clone())),
        "history serves our reply's parent"
    );
    assert_eq!(reply_of(&friends, &gid, &parent).await, Some(None));
    assert!(
        content::send(&friends, &gid, "x", Some("not-a-client-id"))
            .await
            .is_err(),
        "a parent that is not a client id is refused"
    );
}

#[tokio::test]
async fn a_members_reply_to_an_unheld_message_is_stored_and_served_as_is() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let key = group(&friends, owner).await;
    let gid = hex::encode(GID);
    let mut events = host.ctx.journal.subscribe(None);
    let (sink, stream) = (GroupSink::new(&friends, GID), stream_name(&gid));
    let unheld = client_id_text(&[0xAB; 16]);
    let entry = ChainEntry {
        seq: 1,
        ciphertext: DirectBody::GroupContent(sealed(&key, &reply(1, [0xAB; 16]), 1))
            .encode()
            .unwrap(),
    };

    sink.apply(&stream, &bob(), &entry).await.unwrap();

    let id = client_id_text(&[1; 16]);
    assert_eq!(
        reply_of(&friends, &gid, &id).await,
        Some(Some(unheld.clone())),
        "a reply to a message we never received keeps its parent's id"
    );
    let mut published = None;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        if let StreamFrame::Event(seq) = frame {
            if let SubscriptionEvent::GroupDm(GroupDmEvent::Message { reply_to, .. }) = seq.event {
                published = Some(reply_to);
            }
        }
    }
    assert_eq!(published, Some(Some(unheld)));
}

#[tokio::test]
async fn a_tampered_or_forged_reply_is_refused_like_a_tampered_body() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let key = group(&friends, owner).await;
    let gid = hex::encode(GID);
    let (sink, stream) = (GroupSink::new(&friends, GID), stream_name(&gid));

    let mut tampered = sealed(&key, &reply(1, [0xAB; 16]), 1);
    let last = tampered.sealed.len() - 1;
    tampered.sealed[last] ^= 0x01;
    // Sealed by someone without Bob's key, claiming to be Bob.
    let forger = sender_key::mint_first(&OsRandom);
    let forged = sealed(&forger, &reply(2, [0xCD; 16]), 2);
    for (seq, content) in [(1, tampered), (2, forged)] {
        let entry = ChainEntry {
            seq,
            ciphertext: DirectBody::GroupContent(content).encode().unwrap(),
        };
        sink.apply(&stream, &bob(), &entry).await.unwrap();
    }
    for n in [1, 2] {
        assert_eq!(
            reply_of(&friends, &gid, &client_id_text(&[n; 16])).await,
            None,
            "entry {n} is not stored"
        );
    }
    assert_eq!(sink.cursor(&stream, &bob()).await.unwrap(), 2, "skipped");
}
