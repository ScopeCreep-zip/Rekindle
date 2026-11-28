//! CP1: readers authorize the signed post cut, including when the cut
//! arrives separately from the post.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::device_signer::{own_op_record, OpSigner};
use rekindle_governance_runtime::op_log::{self, Deps, OpFate};
use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::{Fault, Writes};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions::SEND_MESSAGES;
use serde_json::Value;

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

async fn post_row(community: &Community, reader: usize, id: &str) -> Option<Value> {
    community
        .history(reader, None, 100)
        .await
        .ok()?
        .into_iter()
        .find(|row| row["serverMessageId"] == id)
}

fn message_id(post: &Value) -> &str {
    post["messageId"].as_str().expect("message id")
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
                })
        })
        .await,
        "the later cut includes the revoked role"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_channel_post_stays_pending_until_its_governance_cut_resolves() {
    let community = Community::of(2, 972).await;
    let anchor = &community.anchor;
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

    let posted = community
        .post(0, "body waits for its signed cut")
        .await
        .unwrap();
    let id = message_id(&posted);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            post_row(&community, 1, id).await.is_some_and(|row| {
                row["body"] == ""
                    && row["admission"]
                        == serde_json::json!({
                            "status": "pending",
                            "text": "Waiting to verify that this post was allowed."
                        })
            })
        })
        .await,
        "the reader exposes a pending placeholder without opening the body"
    );
    assert!(post_row(&community, 1, id)
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
            post_row(&community, 1, id).await.is_some_and(|row| {
                row["body"] == "body waits for its signed cut" && row["admission"].is_null()
            })
        })
        .await,
        "the same id resolves to the accepted post"
    );
}
