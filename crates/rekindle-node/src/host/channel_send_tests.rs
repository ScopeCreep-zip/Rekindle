//! The Host's channel send (plan E3.1 E2b), offline: a member without
//! `SEND_MESSAGES` is refused before anything is stored or written, and a
//! send inside the channel's slowmode is refused.

use rekindle_channel::error::ChannelError;
use rekindle_governance::state::GovernanceState;
use rekindle_types::message::SendState;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::{refusal, HostChannel};
use crate::host::Host;

const SEND_CHANNEL: &str = "0123456789abcdef0123456789abcdef";

/// A text channel in Ann's `c1`, with its record key, its slowmode and our
/// last send in it, stored as a join stores them and loaded into the
/// community store by the unlock's restore (which derives our pseudonym);
/// the merged governance grants nobody anything.
async fn add_channel(
    host: &Host,
    owner: &str,
    slowmode_seconds: Option<u32>,
    last_send_ms: Option<i64>,
) {
    let o = owner.to_string();
    (host.ctx.db.current().unwrap())
        .call(move |conn| {
            rekindle_db::repo::channels::insert(
                conn,
                &o,
                "c1",
                &rekindle_db::repo::channels::StoredChannel {
                    id: SEND_CHANNEL,
                    name: "general",
                    channel_type: "text",
                    category_id: None,
                    topic: "",
                    slowmode_seconds: slowmode_seconds.unwrap_or(0),
                    nsfw: false,
                    mek_generation: 0,
                    parent_voice_channel_id: None,
                },
            )?;
            match last_send_ms {
                Some(at) => {
                    rekindle_db::repo::channels::set_last_send(conn, &o, "c1", SEND_CHANNEL, at)
                }
                None => Ok(()),
            }
        })
        .await
        .unwrap();
    let secret = host.ctx.identity_secret().expect("an unlocked identity");
    crate::host::adapters::governance::loader::restore(&host.ctx, owner, &secret)
        .await
        .unwrap();
    (host.ctx.communities).set_governance_state(
        "c1",
        super::channel_gap_tests::everyone(GovernanceState::default()),
    );
}

fn code(response: &rekindle_ipc::IpcResponse) -> u32 {
    match response {
        rekindle_ipc::IpcResponse::Error { code, .. } => *code,
        other => panic!("not a refusal: {other:?}"),
    }
}

async fn stored_rows(host: &Host, owner: &str) -> usize {
    let owner = owner.to_string();
    let db = host.ctx.db.current().unwrap();
    db.call(move |conn| {
        rekindle_db::repo::messages::channel::page(conn, &owner, SEND_CHANNEL, None, 10)
    })
    .await
    .unwrap()
    .len()
}

#[tokio::test]
async fn a_send_without_send_messages_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    add_channel(&host, &owner, None, None).await;
    let refused = HostChannel::new(&host.ctx)
        .send("c1", SEND_CHANNEL, "hello", None)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, ChannelError::PermissionDenied(_)),
        "{refused:?}"
    );
    assert_eq!(code(&refusal(&refused)), 403);
    assert_eq!(stored_rows(&host, &owner).await, 0, "nothing is stored");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_send_inside_the_slowmode_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let now = host.ctx.clock.now_ms_i64();
    add_channel(&host, &owner, Some(30), Some(now - 1_000)).await;
    let channel = HostChannel::new(&host.ctx);
    let refused = rekindle_channel::enforce_slowmode_with_bypass(&channel, "c1", SEND_CHANNEL, now)
        .unwrap_err();
    assert!(refused.to_string().contains("slowmode active"), "{refused}");
    assert_eq!(code(&refusal(&refused)), 429);
    let later = now + 31_000;
    assert!(
        rekindle_channel::enforce_slowmode_with_bypass(&channel, "c1", SEND_CHANNEL, later).is_ok()
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_reply_is_stored_against_its_parent_row() {
    use rekindle_channel::deps::{ChannelMessagingDeps as _, ChannelSendOutcome};
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    add_channel(&host, &owner, None, None).await;
    let channel = HostChannel::new(&host.ctx);
    let sent = |message_id: &str, reply_to: Option<&str>| ChannelSendOutcome {
        message_id: message_id.into(),
        sender_pseudonym_hex: owner.clone(),
        ciphertext: vec![1],
        mek_generation: Some(1),
        timestamp_ms: 1,
        reply_to: reply_to.map(str::to_string),
        send_state: rekindle_types::message::SendState::Queued,
    };
    for (id, parent, body) in [
        ("msg_parent", None, "first"),
        ("msg_reply", Some("msg_parent"), "second"),
    ] {
        channel
            .persist_sent_message("c1", SEND_CHANNEL, &sent(id, parent), body)
            .await
            .unwrap();
    }
    let o = owner.clone();
    let (parent_row, linked) = (host.ctx.db.current().unwrap())
        .call(move |conn| {
            let parent_row = rekindle_db::repo::messages::channel::row_id_by_message_id(
                conn,
                &o,
                SEND_CHANNEL,
                "msg_parent",
            )?;
            let linked = rekindle_db::repo::messages::channel::reply_row_id(
                conn,
                &o,
                SEND_CHANNEL,
                "msg_reply",
            )?;
            Ok((parent_row, linked))
        })
        .await
        .unwrap();
    assert!(parent_row.is_some(), "the parent is stored");
    assert_eq!(linked, parent_row, "reply_to_id is the parent's row");
    let retry_source = channel
        .find_channel_message_by_id(SEND_CHANNEL, "msg_reply")
        .await;
    assert_eq!(
        retry_source.and_then(|row| row.reply_to).as_deref(),
        Some("msg_parent"),
        "the Retry re-sends the parent's id"
    );
    host.shutdown().await.unwrap();
}

async fn post_state(host: &Host, owner: &str, id: &'static str) -> Option<SendState> {
    let owner = owner.to_string();
    (host.ctx.db.current().unwrap())
        .call(move |c| rekindle_db::repo::messages::send_state(c, &owner, id))
        .await
        .unwrap()
}

/// Plan E3.2 S7l (F9-1): a key's install releases the posts waiting for it
/// one release at a time. While another release holds the lock, the post
/// stays waiting; once it is free, the post leaves `awaiting_key` for its
/// stream (offline, the sender defers it; never left waiting).
#[tokio::test]
async fn a_release_of_waiting_posts_waits_for_the_one_running() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    add_channel(&host, &owner, None, None).await;
    let posts = host.channel_posts().expect("the unlocked outbox");
    let o = owner.clone();
    (host.ctx.db.current().unwrap())
        .call(move |c| {
            rekindle_db::test_support::waiting_channel_post(c, &o, SEND_CHANNEL, "m1");
            Ok(())
        })
        .await
        .unwrap();
    let post = rekindle_types::message::ChannelPost {
        community: "c1".into(),
        channel: SEND_CHANNEL.into(),
        message: "m1".into(),
        entry: rekindle_types::message::PostEntry::Message,
    };
    posts.queue_waiting(&post, i64::MAX / 2).await.unwrap();
    let running = host.ctx.outbox.channel_release_lock();
    let guard = running.lock().await;
    let gov = crate::daemon::dispatch::adapter(&host.ctx);
    let channel_scope = rekindle_types::channel_keys::KeyScope::Channel(
        rekindle_types::id::ChannelId::from_hex(SEND_CHANNEL).unwrap(),
    );
    for scope in [
        rekindle_types::channel_keys::KeyScope::Community,
        channel_scope,
    ] {
        gov.install_transferred_mek(
            "c1",
            scope,
            &rekindle_transport::crypto::mek::Mek::from_bytes([7; 32], 1),
        );
    }
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        post_state(&host, &owner, "m1").await,
        Some(SendState::AwaitingKey),
        "no release runs beside the one holding the lock"
    );
    drop(guard);
    let mut left = false;
    for _ in 0..100 {
        if post_state(&host, &owner, "m1").await != Some(SendState::AwaitingKey) {
            left = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(left, "released once the running release ends");
    host.shutdown().await.unwrap();
}
