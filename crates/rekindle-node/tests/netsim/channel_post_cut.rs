//! CP1: readers authorize the signed post cut, including when the cut
//! arrives separately from the post.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::device_signer::{own_op_record, OpSigner};
use rekindle_governance_runtime::op_log::{self, Deps, OpFate};
use rekindle_ipc::protocol::{
    ChannelRequest, CommunityRequest, IpcRequest, MemberRequest, RichRequest,
};
use rekindle_netsim::{Fault, Writes};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions::SEND_MESSAGES;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};
use serde_json::Value;

use crate::harness::{eventually, Community, Events, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

async fn post_row(community: &Community, reader: usize, id: &str) -> Option<Value> {
    community
        .history(reader, None, 100)
        .await
        .ok()?
        .into_iter()
        .find(|row| row["serverMessageId"] == id)
}

async fn judged_row(community: &Community, id: &str, thread: Option<&str>) -> Option<Value> {
    let Some(thread_id) = thread else {
        return post_row(community, 1, id).await;
    };
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Rich(
            RichRequest::ThreadMessages {
                community: community.anchor.clone(),
                thread_id: thread_id.into(),
                limit: 100,
                before_timestamp: None,
            },
        )))
        .await
        .as_array()?
        .iter()
        .find(|row| row["serverMessageId"] == id)
        .cloned()
}

fn message_id(post: &Value) -> &str {
    post["messageId"].as_str().expect("message id")
}

fn saw_admission(
    events: &Events,
    community: &str,
    stream: &str,
    thread: bool,
    id: &str,
    expected_row: &Value,
) -> bool {
    SimNode::saw(events, |event| {
        let SubscriptionEvent::ChannelMessage(ChannelMessageEvent::PostAdmissionChanged {
            community: event_community,
            stream_id,
            thread: event_thread,
            message_id,
            pending,
            row,
        }) = event
        else {
            return false;
        };
        event_community == community
            && stream_id == stream
            && *event_thread == thread
            && message_id == id
            && *pending == (expected_row["admission"]["status"] == "pending")
            && serde_json::to_value(row).unwrap() == *expected_row
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_channel_post_is_judged_before_or_after_the_role_revocation() {
    let community = Community::of(2, 971).await;
    let anchor = &community.anchor;
    let channel = community.general_id(0).await;
    let bea = community.pseudonym(1).await;
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: anchor.clone(),
                name: "writers".into(),
                permissions: SEND_MESSAGES,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
    let role = role.as_str().unwrap().to_owned();
    for (target, allow, deny) in [
        (RoleId::EVERYONE.to_hex(), 0, SEND_MESSAGES),
        (role.clone(), SEND_MESSAGES, 0),
    ] {
        community.nodes[0]
            .ok(IpcRequest::Community(CommunityRequest::Channels(
                ChannelRequest::ChannelOverwriteSet {
                    community: anchor.clone(),
                    channel_id: channel.clone(),
                    target_type: "role".into(),
                    target_id: target,
                    allow,
                    deny,
                },
            )))
            .await;
    }
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleAssign {
                community: anchor.clone(),
                member_pseudonym: bea.clone(),
                role_id: role.clone(),
            },
        )))
        .await;
    assert!(community.holds_role(1, &role).await);
    let author = community.nodes[1].host.governance();
    let before = author
        .governance_state(anchor)
        .expect("writer sees the grant");

    community.sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Subkey {
            subkey: OUTBOX,
            record: None,
        },
    });
    let prior = community.post(1, "queued before revocation").await.unwrap();
    assert!(post_row(&community, 0, message_id(&prior)).await.is_none());

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleUnassign {
                community: anchor.clone(),
                member_pseudonym: bea.clone(),
                role_id: role.clone(),
            },
        )))
        .await;
    let role_key = RoleId::from_hex(&role).unwrap();
    let author_key = PseudonymKey::from_hex_lossy(&bea);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            author.governance_state(anchor).is_some_and(|state| {
                state
                    .role_assignments
                    .get(&author_key)
                    .is_none_or(|roles| !roles.contains(&role_key))
            })
        })
        .await
    );
    assert!(community
        .post(1, "ordinary local send is refused")
        .await
        .is_err());

    // Keep the real verified op store, but put the writer-side permission
    // projection back before revocation. The signed cut still includes the
    // revocation, and only the reader can prevent this post from appearing.
    author.set_governance_state(anchor, before);
    let events = community.nodes[0].events().await;
    let later = community.post(1, "queued after revocation").await.unwrap();
    community.sim.fault(Fault::Heal);

    assert!(
        eventually(&community.sim, LIMIT, || async {
            post_row(&community, 0, message_id(&prior))
                .await
                .is_some_and(|row| {
                    row["body"] == "queued before revocation" && row["admission"].is_null()
                })
        })
        .await,
        "the earlier cut remains valid after revocation"
    );
    assert!(
        eventually(&community.sim, LIMIT, || async {
            post_row(&community, 0, message_id(&later))
                .await
                .is_some_and(|row| {
                    row["body"] == ""
                        && row["admission"]
                            == serde_json::json!({
                                "status": "refused",
                                "reason": "missingPermission",
                                "reasonText": "This post was refused because the author did not have permission to post here."
                            })
                        && saw_admission(&events, anchor, &channel, false, message_id(&later), &row)
                })
        })
        .await,
        "the refused event carries the same body-free row and reason as history"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_channel_post_stays_pending_until_its_governance_cut_resolves() {
    held_post_publishes_history_rows(false, 972).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_thread_reply_publishes_its_pending_and_admitted_history_rows() {
    held_post_publishes_history_rows(true, 974).await;
}

async fn held_post_publishes_history_rows(thread: bool, seed: u64) {
    let community = Community::of(2, seed).await;
    let channel = community.general_id(0).await;
    let anchor = &community.anchor;
    let thread_id = if thread {
        let starter = community.post(0, "start a thread").await.unwrap();
        let created = community.nodes[0]
            .ok(IpcRequest::Community(CommunityRequest::Rich(
                RichRequest::ThreadCreate {
                    community: anchor.clone(),
                    channel_id: channel.clone(),
                    name: "Admission".into(),
                    starter_message_id: message_id(&starter).into(),
                    forum_tag: None,
                    auto_archive_seconds: None,
                },
            )))
            .await;
        let thread_id = created.as_str().expect("a thread id").to_owned();
        assert!(
            eventually(&community.sim, LIMIT, || async {
                community.nodes[1]
                    .call(IpcRequest::Community(CommunityRequest::Rich(
                        RichRequest::ThreadMessages {
                            community: anchor.clone(),
                            thread_id: thread_id.clone(),
                            limit: 100,
                            before_timestamp: None,
                        },
                    )))
                    .await
                    .is_ok()
            })
            .await
        );
        Some(thread_id)
    } else {
        None
    };
    let stream = thread_id.as_deref().unwrap_or(&channel);
    let author = community.nodes[0].host.governance();
    let reader = community.nodes[1].host.governance();
    let cert = author.genesis_cert(anchor).unwrap();
    let state = author.governance_state(anchor).unwrap();
    let held = author.op_store().governance_ops(anchor).await.unwrap();
    let set = op_log::verify_held_with(&author, anchor, &held);
    let record = own_op_record(&author, anchor, &state).unwrap();
    let key = SigningKey::from_bytes(&author.device_op_key(anchor).unwrap().unwrap());
    let (op, encoded) = op_log::build_op(
        anchor,
        &cert,
        &state,
        &set,
        OpSigner::checked(anchor, &state, cert.creator_pseudonym.clone(), &key).unwrap(),
        GovernanceEntry::CommunityMeta {
            name: None,
            description: Some("a later governance head".into()),
            icon_hash: None,
            banner_hash: None,
        },
        Deps::Cited(Vec::new()),
        1,
    )
    .unwrap();
    let held = op_log::held(&op, record, encoded, OpFate::Applied);
    author
        .op_store()
        .store_governance_op(anchor, &held)
        .await
        .unwrap();

    let events = community.nodes[1].events().await;
    let posted = if let Some(thread_id) = &thread_id {
        community.nodes[0]
            .ok(IpcRequest::Community(CommunityRequest::Rich(
                RichRequest::ThreadSend {
                    community: anchor.clone(),
                    thread_id: thread_id.clone(),
                    body: "body waits for its signed cut".into(),
                },
            )))
            .await
    } else {
        community
            .post(0, "body waits for its signed cut")
            .await
            .unwrap()
    };
    let id = message_id(&posted);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            judged_row(&community, id, thread_id.as_deref())
                .await
                .is_some_and(|row| {
                    row["body"] == ""
                        && row["admission"]
                            == serde_json::json!({
                                "status": "pending",
                                "text": "Waiting to verify that this post was allowed."
                            })
                        && saw_admission(&events, anchor, stream, thread, id, &row)
                })
        })
        .await,
        "the pending event carries the same body-free placeholder as history"
    );
    assert!(judged_row(&community, id, thread_id.as_deref())
        .await
        .is_some_and(|row| row["body"] == ""));

    // Let the last governance op reach the reader, then run the normal
    // governance merge and projection. The post is reconsidered from its
    // original outbox location without the author publishing it twice.
    reader
        .op_store()
        .store_governance_op(anchor, &held)
        .await
        .unwrap();
    let merged = op_log::rebuild_ops(&reader, anchor).await.unwrap();
    reader.set_governance_state(anchor, merged);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            judged_row(&community, id, thread_id.as_deref())
                .await
                .is_some_and(|row| {
                    row["body"] == "body waits for its signed cut"
                        && row["admission"].is_null()
                        && saw_admission(&events, anchor, stream, thread, id, &row)
                })
        })
        .await,
        "the admitted event carries the same row and body as history"
    );
}

#[cfg(all(feature = "webdriver-test", debug_assertions))]
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn webdriver_post_skips_the_author_gate_but_the_reader_refuses_it() {
    let community = Community::of(2, 973).await;
    let channel = community.general_id(0).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteSet {
                community: community.anchor.clone(),
                channel_id: channel.clone(),
                target_type: "role".into(),
                target_id: RoleId::EVERYONE.to_hex(),
                allow: 0,
                deny: SEND_MESSAGES,
            },
        )))
        .await;
    let author = &community.nodes[1];
    let gov = author.host.governance();
    let channel_id = rekindle_types::id::ChannelId::from_hex(&channel).unwrap();
    assert!(
        eventually(&community.sim, LIMIT, || async {
            gov.governance_state(&community.anchor)
                .is_some_and(|state| {
                    state.overwrites.iter().any(|((id, _), overwrite)| {
                        *id == channel_id && overwrite.deny & SEND_MESSAGES != 0
                    })
                })
        })
        .await
    );
    assert_eq!(
        community.post(1, "ordinary post").await.unwrap_err().code,
        403
    );

    let posted = author
        .ok(IpcRequest::WebdriverPostSkippingLocalPermission {
            channel,
            body: "signed by the denied author".into(),
            reply_to: None,
        })
        .await;
    let id = message_id(&posted);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            post_row(&community, 0, id).await.is_some_and(|row| {
                row["body"] == ""
                    && row["admission"]["status"] == "refused"
                    && row["admission"]["reason"] == "missingPermission"
            })
        })
        .await
    );
}

#[cfg(all(feature = "webdriver-test", debug_assertions))]
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn webdriver_governance_hold_makes_a_live_post_pending_until_release() {
    let community = Community::of(2, 974).await;
    let anchor = &community.anchor;
    let reader = &community.nodes[1];
    reader
        .ok(IpcRequest::WebdriverHoldGovernanceOps {
            community: anchor.clone(),
        })
        .await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelSetTopic {
                community: anchor.clone(),
                channel_id: community.general_id(0).await,
                topic: "creates a new governance head".into(),
            },
        )))
        .await;

    let body = "post made after the topic update";
    let posted = community.post(0, body).await.unwrap();
    let id = message_id(&posted);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            post_row(&community, 1, id)
                .await
                .is_some_and(|row| row["body"] == "" && row["admission"]["status"] == "pending")
        })
        .await
    );
    // Wait for a normal receive path to fetch the op while the hold is active.
    for _ in 0..30 {
        community.sim.quiesce().await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert_eq!(
        post_row(&community, 1, id).await.unwrap()["admission"]["status"],
        "pending"
    );

    let released = reader
        .ok(IpcRequest::WebdriverReleaseGovernanceOps {
            community: anchor.clone(),
        })
        .await;
    assert!(released["released"].as_u64().unwrap() > 0);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            post_row(&community, 1, id)
                .await
                .is_some_and(|row| row["body"] == body && row["admission"].is_null())
        })
        .await
    );
}
