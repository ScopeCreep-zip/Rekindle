//! FT1 (plan E3.3): `CreateForumPost` through the Host on two members.
//! A post is the thread and its starter as one outcome; thread creation
//! takes `CREATE_PUBLIC_THREADS` and a reply `SEND_IN_THREADS` on the
//! channel (both in the default role); a refused post creates nothing; a
//! post made while a member is offline reaches it after it reconnects.

use std::time::Duration;

use rekindle_db::test_support::forum_posts;
use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest, RichRequest};
use rekindle_netsim::Fault;
use rekindle_types::message::SendState;
use rekindle_types::permissions::{CREATE_PUBLIC_THREADS, SEND_IN_THREADS};
use rekindle_types::subscription_events::{ChannelMessageEvent, SocialEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events, Refused};

const LIMIT: Duration = Duration::from_secs(180);
/// Long enough for a governance change to reach the other member.
const SETTLE: Duration = Duration::from_secs(90);

/// Ann opens the forum "ideas"; its id once both members hold it.
async fn forum(community: &Community) -> String {
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.anchor.clone(),
                name: "ideas".into(),
                kind: "forum".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
    let id_on = |info: serde_json::Value| {
        info["channels"].as_array().and_then(|channels| {
            channels
                .iter()
                .find(|c| c["name"] == "ideas")
                .and_then(|c| c["id"].as_str().map(str::to_owned))
        })
    };
    let held = eventually(&community.sim, LIMIT, || async {
        id_on(community.info(1).await).is_some()
    })
    .await;
    assert!(held, "bea holds ann's forum");
    id_on(community.info(0).await).expect("the forum's id")
}

async fn post(
    community: &Community,
    i: usize,
    forum: &str,
    title: &str,
) -> Result<serde_json::Value, Refused> {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Rich(
            RichRequest::CreateForumPost {
                community: community.anchor.clone(),
                channel_id: forum.to_owned(),
                title: title.to_owned(),
                forum_tag: None,
                body: format!("{title} body"),
            },
        )))
        .await
}

async fn reply(
    community: &Community,
    i: usize,
    thread: &str,
) -> Result<serde_json::Value, Refused> {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Rich(
            RichRequest::ThreadSend {
                community: community.anchor.clone(),
                thread_id: thread.to_owned(),
                body: "a reply".into(),
            },
        )))
        .await
}

/// The thread names node `i` lists in `forum`.
async fn titles(community: &Community, i: usize, forum: &str) -> Vec<String> {
    let listed = community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Rich(
            RichRequest::ThreadList {
                community: community.anchor.clone(),
                channel_id: forum.to_owned(),
            },
        )))
        .await;
    listed
        .as_array()
        .expect("a thread list")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect()
}

/// Whether node `i` reads `body` in `thread`.
async fn reads(community: &Community, i: usize, thread: &str, body: &str) -> bool {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Rich(
            RichRequest::ThreadMessages {
                community: community.anchor.clone(),
                thread_id: thread.to_owned(),
                limit: 50,
                before_timestamp: None,
            },
        )))
        .await
        .is_ok_and(|page| page.to_string().contains(body))
}

async fn deny(community: &Community, forum: &str, bea: &str, bits: u64) {
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteSet {
                community: community.anchor.clone(),
                channel_id: forum.to_owned(),
                target_type: "member".into(),
                target_id: bea.to_owned(),
                allow: 0,
                deny: bits,
            },
        )))
        .await;
    tokio::time::sleep(SETTLE).await;
}

fn thread_of(created: &serde_json::Value) -> String {
    assert_eq!(
        created["starter"], "queued",
        "the starter is in the outbox: {created}"
    );
    created["threadId"]
        .as_str()
        .expect("a thread id")
        .to_owned()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn forum_post_permissions_and_atomic_refusal() {
    let community = Community::of(2, 241).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let forum = forum(&community).await;
    let bea = community.pseudonym(1).await;

    // The default role creates a post and replies.
    let events = community.nodes[1].events().await;
    let first = post(&community, 1, &forum, "maps")
        .await
        .expect("bea posts");
    let thread = thread_of(&first);
    let read = eventually(&community.sim, LIMIT, || async {
        reads(&community, 0, &thread, "maps body").await
    })
    .await;
    assert!(read, "ann reads bea's starter");
    let sent = reply(&community, 1, &thread).await.expect("bea replies");
    assert_eq!(sent["sendState"], "queued");
    assert_send_states(&community, &events, &thread, &first["starterMessageId"]).await;
    assert_send_states(&community, &events, &thread, &sent["messageId"]).await;

    // Without CREATE_PUBLIC_THREADS (View Channel and Send Messages kept):
    // refused, nothing created.
    deny(&community, &forum, &bea, CREATE_PUBLIC_THREADS).await;
    let refused = post(&community, 1, &forum, "blocked").await;
    assert!(
        refused.as_ref().is_err_and(|r| r.code == 403),
        "no CREATE_PUBLIC_THREADS, no post: {refused:?}"
    );
    tokio::time::sleep(SETTLE).await;
    for i in [0, 1] {
        assert!(
            !titles(&community, i, &forum)
                .await
                .iter()
                .any(|t| t == "blocked"),
            "a refused post creates no thread (node {i})"
        );
    }

    // Without SEND_IN_THREADS: no reply, and no post (its starter is one).
    deny(&community, &forum, &bea, SEND_IN_THREADS).await;
    let refused = reply(&community, 1, &thread).await;
    assert!(
        refused.as_ref().is_err_and(|r| r.code == 403),
        "no SEND_IN_THREADS, no reply: {refused:?}"
    );
    let refused = post(&community, 1, &forum, "silent").await;
    assert!(
        refused.as_ref().is_err_and(|r| r.code == 403),
        "no SEND_IN_THREADS, no post: {refused:?}"
    );

    // Both granted again: allowed.
    deny(&community, &forum, &bea, 0).await;
    let replied = reply(&community, 1, &thread)
        .await
        .expect("bea replies again");
    assert_eq!(replied["sendState"], "queued");
    let again = post(&community, 1, &forum, "allowed")
        .await
        .expect("bea posts again");
    thread_of(&again);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn forum_post_reaches_an_offline_member() {
    let community = Community::of(2, 243).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let forum = forum(&community).await;

    community.sim.fault(Fault::Detach {
        node: 1,
        detached: true,
    });
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1],
    });
    let created = post(&community, 0, &forum, "while away")
        .await
        .expect("ann posts while bea is away");
    let thread = thread_of(&created);
    tokio::time::sleep(Duration::from_secs(120)).await;

    community.sim.fault(Fault::Heal);
    community.sim.fault(Fault::Detach {
        node: 1,
        detached: false,
    });
    let delivered = eventually(&community.sim, LIMIT, || async {
        reads(&community, 1, &thread, "while away body").await
    })
    .await;
    assert!(delivered, "bea reads the starter after she reconnects");
}

async fn assert_send_states(
    community: &Community,
    events: &Events,
    thread: &str,
    id: &serde_json::Value,
) {
    let id = id.as_str().expect("message id");
    assert!(eventually(&community.sim, LIMIT, || async {
        [SendState::Queued, SendState::Sent].into_iter().all(|wanted| {
            events.any(|event| matches!(
                event,
                SubscriptionEvent::ChannelMessage(ChannelMessageEvent::ChannelSendStatus {
                    community: scope, channel, message_id, state, reason: None,
                }) if scope == &community.anchor && channel == thread && message_id == id && *state == wanted
            ))
        })
    }).await, "queued and sent status for {id} in {thread}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn forum_post_storage_fault_rolls_back_then_retry_commits_once() {
    let community = Community::of(2, 247).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let forum = forum(&community).await;
    let node = &community.nodes[0];
    let events = node.events().await;
    let db = node.db.current().expect("unlocked database");
    let before = forum_posts::counts(&db, &community.anchor).await.unwrap();
    forum_posts::reject_starter(&db).await.unwrap();

    let error = post(&community, 0, &forum, "retry me")
        .await
        .expect_err("second row fails");
    assert_eq!(error.code, 500);
    assert!(
        error.message.contains("starter insert refused"),
        "{error:?}"
    );
    assert_eq!(
        forum_posts::counts(&db, &community.anchor).await.unwrap(),
        before
    );
    assert!(titles(&community, 0, &forum).await.is_empty());
    community.sim.quiesce().await;
    assert!(!events.any(|event| matches!(
        event,
        SubscriptionEvent::Social(
            SocialEvent::ThreadCreated { .. } | SocialEvent::ThreadMessagePosted { .. }
        ) | SubscriptionEvent::ChannelMessage(ChannelMessageEvent::ChannelSendStatus { .. })
    )));

    forum_posts::reject_echo_instead(&db).await.unwrap();

    let created = post(&community, 0, &forum, "retry me")
        .await
        .expect("echo failure after commit cannot refuse");
    let thread = thread_of(&created);
    let after = forum_posts::counts(&db, &community.anchor).await.unwrap();
    assert_eq!(after.ops, before.ops + 1);
    assert_eq!(after.head_seq, before.head_seq + 1);
    assert_eq!(after.threads, before.threads + 1);
    assert_eq!(after.post_streams, before.post_streams + 1);
    assert_eq!(titles(&community, 0, &forum).await, ["retry me"]);
    assert_send_states(&community, &events, &thread, &created["starterMessageId"]).await;
    let reads_starter = eventually(&community.sim, LIMIT, || async {
        reads(&community, 1, &thread, "retry me body").await
    })
    .await;
    assert!(
        reads_starter,
        "the queued starter landed despite the failed echo"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn forum_post_starter_fails_when_thread_op_is_abandoned() {
    let community = Community::of(2, 249).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let forum = forum(&community).await;
    let node = &community.nodes[0];
    let events = node.events().await;
    let db = node.db.current().expect("unlocked database");
    community.sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });

    let created = post(&community, 0, &forum, "could not land")
        .await
        .expect("durably queued");
    let thread = thread_of(&created);
    let message = created["starterMessageId"].as_str().unwrap();
    let attempts = forum_posts::expire_creation(&db, message).await.unwrap();
    assert_eq!(attempts, 0, "the starter waits for its unlanded op");
    assert!(
        eventually(&community.sim, LIMIT, || async {
            events.any(|event| matches!(
            event,
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::ChannelSendStatus {
                channel, message_id, state: SendState::Failed, reason: Some(reason), ..
            }) if channel == &thread && message_id == message && reason.contains("thread creation")
        ))
        })
        .await,
        "the abandoned dependency failed its starter with a reason"
    );
    assert_eq!(
        forum_posts::counts(&db, &community.anchor)
            .await
            .unwrap()
            .pending_posts,
        0
    );
    assert!(!reads(&community, 1, &thread, "could not land body").await);
}
