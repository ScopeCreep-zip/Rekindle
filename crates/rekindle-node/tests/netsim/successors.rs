//! Plan E3.2 S7s.3e (SS1b): a successor community is a creator-only act
//! (ADR 0011 "Successor community"; MSC4289's creator power). A member
//! the creator granted `MANAGE_COMMUNITY` signs a `CommunitySuccessor`
//! naming a new anchor; it reaches every member on the `governanceOp @74`
//! ingest and no member's merged state keeps it.

use std::time::Duration;

use rekindle_codec::governance_op::decode_op;
use rekindle_governance::op::invite_proof_bytes;
use rekindle_governance::permissions::compute_permissions;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::member_record::{dflt_opaque_record_key, sign_member_record_binding};
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{
    CommunitySuccessor, GovernanceEntry, GovernanceOp, InviteProof, OpRef, SuccessorReason,
};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::MANAGE_COMMUNITY;

use crate::harness::{eventually, Community};
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
