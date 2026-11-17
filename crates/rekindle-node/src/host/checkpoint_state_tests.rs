//! The checkpoint state snapshot the Host serves (plan E3.2 S7c.3, "State
//! snapshot"), offline: answered to an admitted member with the bytes that
//! hash to the checkpoint's `stateRoot`, refused to anyone else.

use rekindle_codec::capnp_envelope::try_decode_community_envelope;
use rekindle_codec::checkpoint_state::{decode_checkpoint_state_response, CheckpointStateRequest};
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::checkpoint::{
    checkpoint_digest, frontier_hash, state_preimage, state_root, state_root_of,
};
use rekindle_governance::merge::{merge, merge_cut};
use rekindle_governance_runtime::op_log::verify_held;
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{AdmissionMode, Checkpoint, GovernanceEntry, OpRef};
use rekindle_types::id::PseudonymKey;

use super::{
    create_ann, meta, offline_host, seed_governance, seed_registry_community, signed_chain,
    CREATOR_RECORD,
};

fn ask(key: &SigningKey, digest: [u8; 32]) -> Vec<u8> {
    CheckpointStateRequest {
        community: "c1".into(),
        height: 1,
        digest,
        offset: 0,
        requester: key.verifying_key().to_bytes(),
    }
    .encode(key)
    .unwrap()
}

/// Plan E3.2 S7c.3: "answered by the Host governance adapter only to an
/// admitted member". The creator, a member, is served the snapshot of the
/// first checkpoint's cut; a key that is no member is refused, though the
/// request is well signed and names the same checkpoint.
#[tokio::test]
async fn a_state_snapshot_request_from_a_non_member_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let key = SigningKey::from_bytes(&[0xbb; 32]);
    let creator = PseudonymKey(key.verifying_key().to_bytes());
    let cert = rekindle_governance::fixture::cert(&creator, AdmissionMode::default());

    // The cut is the creator's first op; the checkpoint op cites it.
    let first = signed_chain("c1", &key, CREATOR_RECORD, vec![meta("C")]);
    let (at_cut, cut) = merge_cut(&cert, &verify_held("c1", &first).ops);
    let checkpoint = Checkpoint {
        height: 1,
        round: 0,
        prev: None,
        cut: vec![OpRef {
            op_hash: first[0].hash,
            author: creator.clone(),
            author_record: CREATOR_RECORD.into(),
            author_seq: 0,
        }],
        frontier_hash: frontier_hash(&[first[0].hash]),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: state_root(&at_cut, &cut),
        mek_generation: 0,
        eligible_set_hash: [0; 32],
        pages: Vec::new(),
    };
    let digest = checkpoint_digest(&checkpoint);
    let ops = signed_chain(
        "c1",
        &key,
        CREATOR_RECORD,
        vec![meta("C"), GovernanceEntry::Checkpoint(checkpoint.clone())],
    );
    let pinned = seed_governance(&host, &ann.public_key, "c1", &key, &ops).await;
    assert_eq!(pinned, cert);
    // The unlock loader holds the pinned cert in memory (`loader.rs`).
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .genesis_cert = Some(cert.clone());
    let merged = merge(&cert, &verify_held("c1", &ops).ops);
    assert!(
        merged.members.contains_key(&creator),
        "the creator is a member"
    );
    host.governance().set_governance_state("c1", merged);

    let reply = host
        .governance()
        .answer_checkpoint_state(&ask(&key, digest))
        .await
        .expect("a member is served");
    let Ok(Some(CommunityEnvelope::Control(ControlPayload::CheckpointStateResponse { response }))) =
        try_decode_community_envelope(&reply)
    else {
        panic!("not a checkpoint state response");
    };
    let response = decode_checkpoint_state_response(&response).unwrap();
    assert_eq!(response.bytes, state_preimage(&at_cut, &cut));
    assert_eq!(response.total as usize, response.bytes.len());
    assert_eq!(state_root_of(&response.bytes), checkpoint.state_root);

    let stranger = SigningKey::from_bytes(&[0xcc; 32]);
    assert!(
        host.governance()
            .answer_checkpoint_state(&ask(&stranger, digest))
            .await
            .is_none(),
        "a non-member is refused"
    );
    host.shutdown().await.unwrap();
}
