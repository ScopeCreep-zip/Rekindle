//! A thread notice is only a hint (finding SEC4): a reply is stored and
//! published only from its authenticated sender's verified outbox entry,
//! under that verified author; a notice naming another author, or one no
//! readable verified entry backs, stores no row and publishes nothing.

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_db::repo::threads;
use rekindle_lifecycle::SessionScope;
use rekindle_types::thread::ThreadInfo;

use super::{sender_reply, ThreadMessageView};
use crate::host::adapters::channel::HostChannel;
use crate::host::test_fixtures::{create_ann, offline_host, seed_registry_community};
use crate::host::Host;

const THREAD: &str = "thr_00000000000000000000000000000004";
const SENDER: &str = "aa11";
const OTHER: &str = "bb22";

fn notice(claimed: &str) -> ControlPayload {
    ControlPayload::ThreadMessageReceived {
        thread_id: THREAD.into(),
        message_id: "tmsg_1".into(),
        sender_pseudonym: claimed.into(),
        timestamp: 5,
        reply_to_id: None,
    }
}

fn view(author: &str, message_id: &str, opened: bool) -> ThreadMessageView {
    ThreadMessageView {
        sender_pseudonym: author.into(),
        body: if opened { "hi".into() } else { String::new() },
        decryption_failed: !opened,
        timestamp_ms: 7_000,
        is_own: false,
        server_message_id: Some(message_id.into()),
        mek_generation: 1,
        stream_seq: 1,
    }
}

/// Ann unlocked with community `c1` and its thread row stored.
async fn host_with_thread(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, String) {
    let host = offline_host(dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let owner = ann.public_key.clone();
    let thread = ThreadInfo {
        id: THREAD.into(),
        channel_id: "05".repeat(16),
        name: "side".into(),
        starter_message_id: "m0".into(),
        creator_pseudonym: SENDER.into(),
        forum_tag: None,
        created_at: 1,
        archived: false,
        auto_archive_seconds: 3600,
        last_message_at: 1,
        message_count: 0,
    };
    let row_owner = owner.clone();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| threads::upsert(conn, &row_owner, "c1", &thread))
        .await
        .unwrap();
    (host, owner)
}

async fn reply_rows(host: &Host, owner: &str) -> u32 {
    let owner = owner.to_string();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| threads::message_rows(conn, &owner, "c1", THREAD))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_notice_naming_another_member_stores_and_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = host_with_thread(&dir).await;
    let scope = SessionScope::closed("test");
    let event = HostChannel::new(&host.ctx)
        .on_thread_control("c1", SENDER, &notice(OTHER), &scope)
        .await;
    assert!(event.is_none(), "no event for a forged author: {event:?}");
    assert_eq!(reply_rows(&host, &owner).await, 0, "no reply row stored");
}

#[tokio::test]
async fn a_notice_no_verified_entry_backs_stores_and_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = host_with_thread(&dir).await;
    let scope = SessionScope::closed("test");
    let event = HostChannel::new(&host.ctx)
        .on_thread_control("c1", SENDER, &notice(SENDER), &scope)
        .await;
    assert!(
        event.is_none(),
        "no event without a verified entry: {event:?}"
    );
    assert_eq!(reply_rows(&host, &owner).await, 0, "no blank reply row");
}

#[test]
fn only_the_senders_opened_entry_for_the_message_is_the_reply() {
    let genuine = sender_reply(
        vec![view(OTHER, "tmsg_1", true), view(SENDER, "tmsg_1", true)],
        "tmsg_1",
        SENDER,
    )
    .expect("the sender's verified entry is the reply");
    assert_eq!(
        genuine.sender_pseudonym, SENDER,
        "shown under its verified author"
    );
    assert_eq!(genuine.body, "hi");
    assert!(
        sender_reply(vec![view(OTHER, "tmsg_1", true)], "tmsg_1", SENDER).is_none(),
        "another author's entry under the same id is not the sender's reply"
    );
    assert!(
        sender_reply(vec![view(SENDER, "tmsg_1", false)], "tmsg_1", SENDER).is_none(),
        "an entry that did not open is never a blank reply"
    );
    assert!(
        sender_reply(vec![view(SENDER, "tmsg_2", true)], "tmsg_1", SENDER).is_none(),
        "another message is not this reply"
    );
}
