//! The frontier the Host serves (plan E3.2 S7d-keys.8, the unlock
//! barrier), offline: answered to an admitted member with our own
//! `Frontier`, refused to anyone else, rate-limited per requester.

use rekindle_codec::capnp_envelope::try_decode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::frontier_request::{decode_frontier_response, FrontierRequest};
use rekindle_governance::merge::merge;
use rekindle_governance_runtime::op_log::verify_held;
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::AdmissionMode;
use rekindle_types::id::PseudonymKey;

use super::{
    create_ann, meta, offline_host, seed_governance, seed_registry_community, signed_chain,
    CREATOR_RECORD,
};

fn ask(key: &SigningKey) -> Vec<u8> {
    FrontierRequest {
        community: "c1".into(),
        requester: key.verifying_key().to_bytes(),
    }
    .encode(key)
    .unwrap()
}

/// Plan E3.2 S7d-keys.8: "answered by the Host governance adapter only to
/// a requester whose envelope signer is an admitted member ... rate-limited
/// per requester like the `headAdvanced` handler". The creator, a member,
/// is served our frontier naming its op; a well-signed stranger is refused;
/// past the per-requester floor the member is refused too.
#[tokio::test]
async fn a_frontier_is_answered_only_to_an_admitted_member_within_its_rate() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let key = SigningKey::from_bytes(&[0xbb; 32]);
    let creator = PseudonymKey(key.verifying_key().to_bytes());
    let cert = rekindle_governance::fixture::cert(&creator, AdmissionMode::default());
    let ops = signed_chain("c1", &key, CREATOR_RECORD, vec![meta("C"), meta("D")]);
    seed_governance(&host, &ann.public_key, "c1", &key, &ops).await;
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .genesis_cert = Some(cert.clone());
    host.governance()
        .set_governance_state("c1", merge(&cert, &verify_held("c1", &ops).ops));

    let stranger = SigningKey::from_bytes(&[0xcc; 32]);
    assert!(
        host.governance()
            .answer_frontier(&ask(&stranger))
            .await
            .is_none(),
        "a non-member is refused"
    );

    let reply = host
        .governance()
        .answer_frontier(&ask(&key))
        .await
        .expect("a member is served");
    let Some(ControlPayload::FrontierResponse { frontier }) = try_decode_community_envelope(&reply)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    else {
        panic!("not a frontier response");
    };
    let frontier = decode_frontier_response(&frontier).unwrap();
    assert_eq!(frontier.heads.len(), 1, "one chain, one tip");
    assert_eq!(frontier.heads[0].op_hash, ops[1].hash);
    assert_eq!(frontier.heads[0].author_seq, 1);
    assert_eq!(frontier.heads[0].author_record, CREATOR_RECORD);

    let mut refused = false;
    for _ in 0..64 {
        if host
            .governance()
            .answer_frontier(&ask(&key))
            .await
            .is_none()
        {
            refused = true;
            break;
        }
    }
    assert!(refused, "a member past its rate is refused");
}

/// Plan E3.2 S7d-keys.8: "only to a requester whose envelope signer is an
/// admitted member". The merged state's `members` is that set (the merge
/// keeps a joiner awaiting approval in `pending_members` and drops a
/// removed member), less `bans` (a ban leaves the member row in place). A
/// joiner awaiting approval, a removed member and a banned member are
/// refused although each has an op record; an approved member is served.
#[tokio::test]
async fn a_frontier_is_refused_to_a_pending_joiner_a_removed_and_a_banned_member() {
    use rekindle_governance::state::{AdmissionDecision, PendingMemberState};
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let key = SigningKey::from_bytes(&[0xbb; 32]);
    let creator = PseudonymKey(key.verifying_key().to_bytes());
    let cert = rekindle_governance::fixture::cert(&creator, AdmissionMode::ApprovalRequired);
    let ops = signed_chain("c1", &key, CREATOR_RECORD, vec![meta("C")]);
    seed_governance(&host, &ann.public_key, "c1", &key, &ops).await;
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .genesis_cert = Some(cert.clone());
    let mut state = merge(&cert, &verify_held("c1", &ops).ops);
    let seat = state
        .members
        .get(&creator)
        .cloned()
        .expect("the creator is seated");
    let pseudo = |k: &SigningKey| PseudonymKey(k.verifying_key().to_bytes());
    let (joiner, removed, banned, approved) = (
        SigningKey::from_bytes(&[0xd1; 32]),
        SigningKey::from_bytes(&[0xd2; 32]),
        SigningKey::from_bytes(&[0xd3; 32]),
        SigningKey::from_bytes(&[0xd4; 32]),
    );
    for k in [&joiner, &removed, &banned, &approved] {
        state
            .op_log_records
            .insert(pseudo(k), seat.member_record.clone());
    }
    state.pending_members.insert(
        pseudo(&joiner),
        PendingMemberState {
            member_record: seat.member_record.clone(),
            member_record_owner: seat.member_record_owner.clone(),
            has_invite: true,
            stamp: seat.joined,
        },
    );
    state.removed.insert(pseudo(&removed), seat.joined);
    state.members.insert(pseudo(&banned), seat.clone());
    state.bans.insert(pseudo(&banned));
    state.members.insert(pseudo(&approved), seat.clone());
    state.admitted.insert(
        pseudo(&approved),
        AdmissionDecision {
            approved: true,
            stamp: seat.joined,
        },
    );
    host.governance().set_governance_state("c1", state);

    for (who, k) in [
        ("pending joiner", &joiner),
        ("removed member", &removed),
        ("banned member", &banned),
    ] {
        assert!(
            host.governance().answer_frontier(&ask(k)).await.is_none(),
            "a {who} is refused"
        );
    }
    assert!(
        host.governance()
            .answer_frontier(&ask(&approved))
            .await
            .is_some(),
        "an approved member is served"
    );
}
