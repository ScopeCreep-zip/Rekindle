//! A join the merge does not admit answers the typed `bootstrapRefused
//! {anchor, reason}` (plan E3.2 S10 Verify (3) and join step 5; the daemon
//! track, `daemon/handler.rs` `BootstrapRequest`: the runtime's
//! `BootstrapAnswer::Refused`, forwarded as is on both hosts (IJ2: the
//! desktop's `network.rs` once NAKed it bare, so the joiner read
//! `NotAdmitted`); any other build failure stays `CallResponse::Rejected`,
//! the transport's `NAK`). An outsider who reaches a member sends a
//! well-formed `BootstrapRequest` whose `MemberJoined` matches its `OpRef`
//! and is signed by its pseudonym, but that the merge does not admit: the
//! answer is the refusal with the reason the merge gave (`JoinRefusal`'s
//! rules), never a `bootstrapResponseV2`, the outsider is no
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
use rekindle_types::config::SafetyProfile;
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, InviteProof, OpRef};
use rekindle_types::id::PseudonymKey;
use rekindle_types::join_refusal::JoinRefusal;

use crate::harness::{eventually, Community};
use crate::scenarios::{signed, vld0};

/// The invite-less outsider: no invite gate names a reason for a join
/// without a proof, so it is one of "every other refusal", `NotAdmitted`
/// (`rekindle-types` `join_refusal.rs`; `bootstrap.rs` `admitted_deps`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_inviteless_outsider_is_refused_as_not_admitted() {
    let community = Community::of(2, 233).await;
    outsider_is_refused_and_not_stored(&community, None, JoinRefusal::NotAdmitted).await;
}

/// An outsider naming a live invite's commitment (public in the op log's
/// `InviteCreated`) with its own proof and owner proof passes the receive
/// bound and verifies, but cites no deps, so the merge's invite gate finds
/// no `InviteCreated` in its causal past and does not admit it: the
/// responder answers `InviteUnknown` (`merge/gates.rs` `invite_gate`: no
/// live invite in the op's causal past matches the proof) and stores
/// nothing.
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
    outsider_is_refused_and_not_stored(
        &community,
        Some(link.invite_code.as_str()),
        JoinRefusal::InviteUnknown,
    )
    .await;
}

async fn outsider_is_refused_and_not_stored(
    community: &Community,
    code: Option<&str>,
    reason: JoinRefusal,
) {
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
            device_key: author.0,
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
                first_device: None,
            },
            sig: [0; 64],
        },
    );
    // The runtime's answer is itself the typed refusal (IJ2), so a host
    // that forwards the answer (the desktop's `network.rs`, the daemon's
    // `handler.rs`) carries the reason; an error a host maps on its own
    // dropped it to a bare NAK, read as `NotAdmitted`.
    let direct = rekindle_governance_runtime::answer_bootstrap_request(
        &community.nodes[0].host.governance(),
        &anchor,
        &rekindle_governance_runtime::BootstrapJoiner {
            pseudonym: hex::encode(author.0),
            member_joined: OpRef {
                op_hash: hash,
                device_key: author.0,
                author: author.clone(),
                author_record: record.clone(),
                author_seq: 0,
            },
            member_joined_op: op.clone(),
            route: Vec::new(),
        },
    )
    .await;
    let Ok(rekindle_governance_runtime::BootstrapAnswer::Refused(bytes)) = direct else {
        panic!("the runtime answers the typed refusal, got {direct:?}");
    };
    assert!(
        matches!(
            rekindle_governance_runtime::bootstrap_peers::read_reply(&bytes, &anchor),
            Err(rekindle_governance_runtime::GovernanceRuntimeError::BootstrapRefused(r)) if r == reason
        ),
        "the refusal names {reason}"
    );
    let request = encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::BootstrapRequest {
            joiner_pseudonym: hex::encode(author.0),
            anchor: anchor.clone(),
            member_joined: OpRef {
                op_hash: hash,
                device_key: author.0,
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
    let refused = rekindle_codec::capnp_envelope::try_decode_community_envelope(&reply);
    assert!(
        matches!(
            &refused,
            Ok(Some(CommunityEnvelope::Control(ControlPayload::BootstrapRefused {
                anchor: answered,
                reason: got,
            }))) if *answered == anchor && *got == reason
        ),
        "the member refuses the join for {reason:?}: {refused:?}"
    );
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
