//! Plan E3.2 S7s.3e (SS1b): a successor community is a creator-only act
//! (ADR 0011 "Successor community"; MSC4289's creator power). A member
//! the creator granted `MANAGE_COMMUNITY` signs a `CommunitySuccessor`
//! naming a new anchor; it reaches every member on the `governanceOp @74`
//! ingest and no member's merged state keeps it.
//!
//! Plan E3.2 S7s.4 (SS3): a creator publishes a successor (its cert names
//! this community as `predecessor`, its op here names the new anchor), so
//! every member is offered it once both links hold; two successors are both
//! offered; a member follows one by joining it and keeps the old community.

use std::time::Duration;

use rekindle_codec::governance_op::decode_op;
use rekindle_governance::op::invite_proof_bytes;
use rekindle_governance::permissions::compute_permissions;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest, MemberRequest};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::member_record::{dflt_opaque_record_key, sign_member_record_binding};
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{
    CommunitySuccessor, GovernanceEntry, GovernanceOp, InviteProof, OpRef, SuccessorReason,
};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::MANAGE_COMMUNITY;

use crate::harness::{eventually, Community, SimNode};
use crate::scenarios::{signed, vld0};

/// The `OpRef` and `issued_at` of the first op in node 0's store whose
/// body matches `pick`.
async fn held_ref(community: &Community, pick: impl Fn(&GovernanceEntry) -> bool) -> (OpRef, u64) {
    let held = (community.nodes[0].host.governance())
        .governance_ops(&community.anchor)
        .await
        .expect("the op store");
    held.iter()
        .find_map(|h| {
            let d = decode_op(&h.encoded).ok()?;
            pick(&d.op.body).then(|| {
                let at = d.op.issued_at;
                let r = OpRef {
                    op_hash: h.hash,
                    author: h.author.clone(),
                    author_record: h.author_record.clone(),
                    author_seq: h.seq,
                };
                (r, at)
            })
        })
        .expect("the op is held")
}

/// Waits until every node's merged state satisfies `check`.
async fn everywhere(
    community: &Community,
    what: &str,
    check: impl Fn(&rekindle_governance::state::GovernanceState) -> bool,
) {
    let all = eventually(&community.sim, Duration::from_secs(180), || async {
        community.nodes.iter().all(|node| {
            (node.host.governance().governance_state(&community.anchor))
                .is_some_and(|state| check(&state))
        })
    })
    .await;
    assert!(all, "every member: {what}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_manager_successor_is_quarantined_on_every_peer() {
    let mut community = Community::of(2, 461).await;
    community.invite().await;
    let link = rekindle_types::invite::InviteLink::parse(&community.link).expect("an invite link");
    let code = link.invite_code.as_str().to_owned();
    let code_hash = rekindle_secrets::invite::hash_invite_code(&code);
    everywhere(&community, "merged the invite", |s| {
        s.invites.values().any(|i| i.code_hash == code_hash)
    })
    .await;
    let anchor = community.anchor.clone();
    let (invite, invite_at) = held_ref(
        &community,
        |b| matches!(b, GovernanceEntry::InviteCreated { code_hash: c, .. } if *c == code_hash),
    )
    .await;

    // A key the test holds joins through the real invite.
    let manager = rekindle_secrets::derive::derive_community_pseudonym(&[0x46; 32], &anchor);
    let author = PseudonymKey(manager.verifying_key().to_bytes());
    let owner = SigningKey::from_bytes(&[0x47; 32])
        .verifying_key()
        .to_bytes();
    let record = vld0(&dflt_opaque_record_key(&owner, member_record::SUBKEY_COUNT));
    let joined = GovernanceOp {
        author: author.clone(),
        seq: 0,
        prev: None,
        deps: vec![invite],
        issued_at: invite_at,
        body: GovernanceEntry::MemberJoined {
            member_record: record.clone(),
            member_record_owner: vld0(&owner),
            invite_proof: Some(InviteProof {
                code_hash: code_hash.clone(),
                sig: rekindle_secrets::invite::sign_invite_join(
                    &code,
                    &invite_proof_bytes(&anchor, &author, &record, &code_hash),
                )
                .to_vec(),
            }),
            owner_proof: sign_member_record_binding(&[0x47; 32], &anchor, &author.0, &record)
                .to_vec(),
            first_device: None,
        },
        sig: [0; 64],
    };
    let (joined, joined_hash) = signed(&anchor, &manager, joined);
    for node in &community.nodes {
        (node.host.governance())
            .on_governance_op(&anchor, joined.clone())
            .await;
    }
    everywhere(&community, "admitted the test key", |s| {
        s.members.contains_key(&author)
    })
    .await;

    // The creator grants it MANAGE_COMMUNITY over IPC.
    let members = |request| IpcRequest::Community(CommunityRequest::Members(request));
    let role = community.nodes[0]
        .ok(members(MemberRequest::RoleCreate {
            community: anchor.clone(),
            name: "stewards".into(),
            permissions: MANAGE_COMMUNITY,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        }))
        .await;
    let role_id = role.as_str().expect("a role id").to_owned();
    community.nodes[0]
        .ok(members(MemberRequest::RoleAssign {
            community: anchor.clone(),
            member_pseudonym: author.to_hex(),
            role_id,
        }))
        .await;
    everywhere(&community, "the test key manages the community", |s| {
        compute_permissions(&author, None, s, 0) & MANAGE_COMMUNITY != 0
    })
    .await;
    let (granted, granted_at) = held_ref(
        &community,
        |b| matches!(b, GovernanceEntry::RoleAssignment { target, .. } if *target == author),
    )
    .await;

    // The manager names a successor at a causal position where it holds
    // MANAGE_COMMUNITY.
    let successor = GovernanceOp {
        author: author.clone(),
        seq: 1,
        prev: Some(joined_hash),
        // The author's previous op is among the deps (`BadChain` otherwise).
        deps: vec![
            OpRef {
                op_hash: joined_hash,
                author: author.clone(),
                author_record: record.clone(),
                author_seq: 0,
            },
            granted,
        ],
        issued_at: granted_at,
        body: GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
            new_anchor: Some(vld0(&[0x5A; 32])),
            reason: SuccessorReason::Recreate,
            transfer_to: None,
            creator_set_hash: None,
            accepts: None,
            links: Vec::new(),
        }),
        sig: [0; 64],
    };
    let (successor, successor_hash) = signed(&anchor, &manager, successor);
    for node in &community.nodes {
        (node.host.governance())
            .on_governance_op(&anchor, successor.clone())
            .await;
    }
    community.sim.quiesce().await;

    for (i, node) in community.nodes.iter().enumerate() {
        let held = (node.host.governance().governance_ops(&anchor).await).expect("the op store");
        assert!(
            held.iter().any(|op| op.hash == successor_hash),
            "node {i} holds the manager's successor, so the merge judged it"
        );
        let state = (node.host.governance().governance_state(&anchor)).expect("a state");
        assert!(
            state.successors.is_empty(),
            "node {i} keeps no successor signed by a manager"
        );
    }
}

fn lifecycle(request: LifecycleRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(request))
}

/// The successors `node` was offered for `anchor` (the snapshot a window
/// reads, lead decision (33)).
async fn offered_to(node: &SimNode, anchor: &str) -> Vec<serde_json::Value> {
    let request = lifecycle(LifecycleRequest::CommunitySuccessors {
        anchor: anchor.to_owned(),
    });
    (node.call(request).await.ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// Node 0, the creator, publishes a successor; its new anchor.
async fn publish(community: &Community) -> String {
    let created = community.nodes[0]
        .ok(lifecycle(LifecycleRequest::CommunityPublishSuccessor {
            anchor: community.anchor.clone(),
        }))
        .await;
    created["anchor"]
        .as_str()
        .expect("the successor's anchor")
        .to_owned()
}

/// Waits until every member but the signer (node 0) is offered each of
/// `anchors`, and returns the offers each holds.
async fn offered_to_members(
    community: &Community,
    anchors: &[String],
) -> Vec<Vec<serde_json::Value>> {
    let all = eventually(&community.sim, Duration::from_secs(300), || async {
        for node in &community.nodes[1..] {
            let offers = offered_to(node, &community.anchor).await;
            let held = |a: &String| offers.iter().any(|o| o["newAnchor"] == a.as_str());
            if !anchors.iter().all(held) {
                return false;
            }
        }
        true
    })
    .await;
    assert!(all, "every member is offered {anchors:?}");
    let mut offers = Vec::new();
    for node in &community.nodes[1..] {
        offers.push(offered_to(node, &community.anchor).await);
    }
    offers
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_creator_successor_is_offered_to_every_member_once_both_links_hold() {
    let community = Community::of(3, 462).await;
    for node in &community.nodes {
        assert!(
            offered_to(node, &community.anchor).await.is_empty(),
            "nothing is offered before a successor exists"
        );
    }
    let new_anchor = publish(&community).await;
    for offers in offered_to_members(&community, std::slice::from_ref(&new_anchor)).await {
        assert_eq!(offers.len(), 1, "one successor, offered once: {offers:?}");
        let offer = &offers[0];
        assert_eq!(offer["reason"], "recreate");
        assert_eq!(
            offer["creators"].as_array().map(Vec::len),
            Some(1),
            "the publisher is the successor's sole creator"
        );
        let creator_here = (community.nodes[0].host.governance())
            .community_membership(&community.anchor)
            .and_then(|m| m.my_pseudonym_hex)
            .expect("the creator's pseudonym");
        assert_eq!(
            offer["signer"],
            creator_here.as_str(),
            "who signed it is shown"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn two_successors_are_both_offered() {
    let community = Community::of(2, 463).await;
    let first = publish(&community).await;
    let second = publish(&community).await;
    assert_ne!(first, second);
    for offers in offered_to_members(&community, &[first, second]).await {
        assert_eq!(offers.len(), 2, "both successors are offered: {offers:?}");
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn follow_joins_the_new_community_and_keeps_the_old_readable() {
    let community = Community::of(2, 464).await;
    let new_anchor = publish(&community).await;
    offered_to_members(&community, std::slice::from_ref(&new_anchor)).await;
    let invite = community.nodes[0]
        .ok(lifecycle(LifecycleRequest::InviteCreate {
            community: new_anchor.clone(),
            max_uses: 1,
            expires_seconds: None,
        }))
        .await;
    let invite = invite["url"].as_str().expect("an invite link").to_owned();
    community.nodes[1]
        .ok(lifecycle(LifecycleRequest::CommunityFollowSuccessor {
            anchor: community.anchor.clone(),
            new_anchor: new_anchor.clone(),
            invite,
        }))
        .await;
    let both = eventually(&community.sim, Duration::from_secs(120), || async {
        (community.nodes[1]
            .call(lifecycle(LifecycleRequest::CommunityList))
            .await)
            .is_ok_and(|v| {
                let listed = v.to_string();
                listed.contains(&new_anchor) && listed.contains(&community.anchor)
            })
    })
    .await;
    assert!(
        both,
        "the follower lists the successor and keeps the old community"
    );
    let old = (community.nodes[1].host.governance()).governance_state(&community.anchor);
    assert!(old.is_some(), "the old community stays readable");
}
