//! A bootstrap build failure answers a NAK on the wire (plan E3.2 S10
//! Verify (3); the daemon track, `daemon/handler.rs` `BootstrapRequest` →
//! `CallResponse::Rejected`, mapped to `NAK` by the transport's
//! `dispatch_bare_request`). An outsider who reaches a member sends a
//! well-formed `BootstrapRequest` whose `MemberJoined` matches its `OpRef`
//! and is signed by its pseudonym, but carries no invite proof: the member
//! cannot merge it as a member (`answer_bootstrap_request` fails), so the
//! answer is the NAK, never a `bootstrapResponseV2`, and the outsider is no
//! member of the member's state.

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::member_record::dflt_opaque_record_key;
use rekindle_transport::operations::direct::NAK;
use rekindle_types::config::SafetyProfile;
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::harness::Community;
use crate::scenarios::{signed, vld0};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_bootstrap_build_failure_answers_a_nak() {
    let community = Community::of(2, 233).await;
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
                invite_proof: None,
                owner_proof: Vec::new(),
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
    let state = community.nodes[0]
        .host
        .governance()
        .governance_state(&anchor)
        .expect("the member holds the community");
    assert!(
        !state.members.contains_key(&author),
        "the refused joiner is no member"
    );
}
