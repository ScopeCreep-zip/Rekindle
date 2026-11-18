//! A bootstrap build failure answers a NAK on the wire (plan E3.2 S10
//! Verify (3); the daemon track, `daemon/handler.rs` `BootstrapRequest` →
//! `CallResponse::Rejected`, mapped to `NAK` by the transport's
//! `dispatch_bare_request`). An outsider who reaches a member sends a
//! well-formed `BootstrapRequest` whose `MemberJoined` matches its `OpRef`
//! and is signed by its pseudonym, but that the merge does not admit: the
//! answer is the NAK, never a `bootstrapResponseV2`, the outsider is no
//! member of the member's state, and the member's op store does not hold
//! the outsider's op (plan E3.2 S10 Found (b): the responder merges in
//! memory before it stores, `answer_bootstrap_request`).

use std::time::Duration;

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::op::invite_proof_bytes;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::member_record::{dflt_opaque_record_key, sign_member_record_binding};
use rekindle_transport::operations::direct::NAK;
use rekindle_types::config::SafetyProfile;
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, InviteProof, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::harness::{eventually, Community};
use crate::scenarios::{signed, vld0};

/// The invite-less outsider: refused by the receive bound already.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_bootstrap_build_failure_answers_a_nak() {
    let community = Community::of(2, 233).await;
    outsider_is_refused_and_not_stored(&community, None).await;
}

/// An outsider naming a live invite's commitment (public in the op log's
/// `InviteCreated`) with its own proof and owner proof passes the receive
/// bound and verifies, but cites no deps, so the merge's invite gate finds
/// no `InviteCreated` in its causal past and does not admit it: the
/// responder answers NAK and stores nothing.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unadmitted_joiner_with_an_invite_commitment_is_not_stored() {
    let mut community = Community::of(2, 234).await;
    community.invite().await;
    let link = rekindle_types::invite::InviteLink::parse(&community.link).expect("an invite link");
    let code_hash = rekindle_secrets::invite::hash_invite_code(link.invite_code.as_str());
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            (community.nodes[0]
                .host
                .governance()
                .governance_state(&community.anchor))
            .is_some_and(|state| state.invites.values().any(|i| i.code_hash == code_hash))
        })
        .await,
        "the member merged the invite"
    );
    outsider_is_refused_and_not_stored(&community, Some(link.invite_code.as_str())).await;
}

async fn outsider_is_refused_and_not_stored(community: &Community, code: Option<&str>) {
    let anchor = community.anchor.clone();
    let outsider = community.sim.add_node();
    community.sim.set_ready(outsider.index, true);
    let blob = community
        .sim
        .route_blobs(0)
        .into_iter()
        .next()
        .expect("the member has a route");
    let target = outsider.ports.routes.import(&blob).unwrap();

    let joiner = rekindle_secrets::derive::derive_community_pseudonym(&[0xB7; 32], &anchor);
    let author = PseudonymKey(joiner.verifying_key().to_bytes());
    let owner = SigningKey::from_bytes(&[0xB8; 32])
        .verifying_key()
        .to_bytes();
    let record = vld0(&dflt_opaque_record_key(&owner, member_record::SUBKEY_COUNT));
    let invite_proof = code.map(|code| {
        let code_hash = rekindle_secrets::invite::hash_invite_code(code);
        let proof = invite_proof_bytes(&anchor, &author, &record, &code_hash);
        InviteProof {
            sig: rekindle_secrets::invite::sign_invite_join(code, &proof).to_vec(),
            code_hash,
        }
    });
    let (op, hash) = signed(
        &anchor,
        &joiner,
        GovernanceOp {
            author: author.clone(),
            seq: 0,
            prev: None,
            deps: Vec::new(),
            issued_at: 0,
            body: GovernanceEntry::MemberJoined {
                member_record: record.clone(),
                member_record_owner: vld0(&owner),
                invite_proof,
                owner_proof: sign_member_record_binding(&[0xB8; 32], &anchor, &author.0, &record)
                    .to_vec(),
            },
            sig: [0; 64],
        },
    );
    let request = encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::BootstrapRequest {
            joiner_pseudonym: hex::encode(author.0),
            anchor: anchor.clone(),
            member_joined: OpRef {
                op_hash: hash,
                author: author.clone(),
                author_record: record,
                author_seq: 0,
            },
            member_joined_op: op,
            joiner_route: community.sim.route_blobs(outsider.index).concat(),
        },
    ))
    .unwrap();
    let reply = outsider
        .ports
        .messages
        .call(&target, &SafetyProfile::default_dht(), request)
        .await
        .expect("the member answers the call");
    assert_eq!(reply, NAK, "a bootstrap the member cannot build is a NAK");
    community.sim.quiesce().await;
    let governance = community.nodes[0].host.governance();
    let state = governance
        .governance_state(&anchor)
        .expect("the member holds the community");
    assert!(
        !state.members.contains_key(&author),
        "the refused joiner is no member"
    );
    let held = governance.governance_ops(&anchor).await.unwrap();
    assert!(
        held.iter().all(|op| op.hash != hash),
        "the member's op store does not hold the refused outsider's op"
    );
}
