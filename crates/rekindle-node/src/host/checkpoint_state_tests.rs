//! The checkpoint state snapshot the Host serves (plan E3.2 S7c.3, "State
//! snapshot"), offline: answered to an admitted member with the bytes that
//! hash to the checkpoint's `stateRoot`, refused to anyone else.

use rekindle_codec::capnp_envelope::try_decode_community_envelope;
use rekindle_codec::checkpoint_state::{
    decode_checkpoint_state_response, CheckpointStateRequest, PreJoinRequest,
};
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
    ask_with(key, digest, None)
}

fn state_request(key: &SigningKey, digest: [u8; 32]) -> CheckpointStateRequest {
    CheckpointStateRequest {
        community: "c1".into(),
        height: 1,
        digest,
        offset: 0,
        requester: key.verifying_key().to_bytes(),
        invite: None,
    }
}

fn ask_with(key: &SigningKey, digest: [u8; 32], invite_code: Option<&str>) -> Vec<u8> {
    let request = state_request(key, digest);
    match invite_code {
        Some(code) => request.with_invite_code(code),
        None => request,
    }
    .encode(key)
    .unwrap()
}

/// Plan E3.2 S7c.3: "answered by the Host governance adapter only to an
/// admitted member". The creator, a member, is served the snapshot of the
/// first checkpoint's cut; a key that is no member is refused, though the
/// request is well signed and names the same checkpoint.
#[tokio::test]
async fn a_state_snapshot_request_from_a_non_member_without_a_live_invite_is_refused() {
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
        state_size: u32::try_from(state_preimage(&at_cut, &cut).len()).expect("fits"),
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

    // Plan E3.2 S7c.5: a joiner holding a live invite is served before it
    // joins; a used-up, expired or revoked invite, or a banned joiner, is
    // refused. The invite is one we minted, so we hold its code.
    let code = "c0de".repeat(8);
    let code_hash = rekindle_secrets::invite::hash_invite_code(&code);
    let owner = host.ctx.require_owner().unwrap();
    let (stored_code, stored_hash) = (code.clone(), code_hash.clone());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| {
            rekindle_db::repo::community_invites::insert(
                c,
                &owner,
                "c1",
                &stored_code,
                &stored_hash,
                1,
            )
        })
        .await
        .unwrap();
    let joiner = |invite: Option<rekindle_governance::state::InviteState>, banned: bool| {
        let mut state = merge(&cert, &verify_held("c1", &ops).ops);
        state.invites.extend(invite.map(|i| ([9; 16], i)));
        if banned {
            state
                .bans
                .insert(PseudonymKey(stranger.verifying_key().to_bytes()));
        }
        host.governance().set_governance_state("c1", state);
    };
    let live = rekindle_governance::state::InviteState {
        code_hash: code_hash.clone(),
        max_uses: 1,
        expires_at: None,
        secrets_record_key: String::new(),
        created: rekindle_governance::op::OpStamp::MIN,
        creator_pseudonym: creator.clone(),
        uses: 0,
    };
    let request = ask_with(&stranger, digest, Some(&code));
    let served = || async { host.governance().answer_checkpoint_state(&request).await };
    joiner(Some(live.clone()), false);
    assert!(
        served().await.is_some(),
        "a joiner with a live invite is served"
    );
    // Plan E3.2 S7c.8 (2): the code hash is public (any reader of the op
    // log has it), so naming it without a tag made from the code is
    // refused, as is a tag under another code.
    let hash_only = CheckpointStateRequest {
        invite: Some(rekindle_codec::checkpoint_state::PreJoinInvite {
            code_hash: code_hash.clone(),
            tag: [0; 32],
        }),
        ..state_request(&stranger, digest)
    }
    .encode(&stranger)
    .unwrap();
    assert!(
        host.governance()
            .answer_checkpoint_state(&hash_only)
            .await
            .is_none(),
        "the public code hash without the code is refused"
    );
    let mut wrong = state_request(&stranger, digest).with_invite_code(&"0bad".repeat(8));
    if let Some(invite) = wrong.invite.as_mut() {
        invite.code_hash.clone_from(&code_hash);
    }
    assert!(
        host.governance()
            .answer_checkpoint_state(&wrong.encode(&stranger).unwrap())
            .await
            .is_none(),
        "a tag under another code is refused"
    );
    joiner(Some(live.clone()), true);
    assert!(served().await.is_none(), "a banned joiner is refused");
    joiner(
        Some(rekindle_governance::state::InviteState {
            uses: 1,
            ..live.clone()
        }),
        false,
    );
    assert!(served().await.is_none(), "a used-up invite is refused");
    joiner(
        Some(rekindle_governance::state::InviteState {
            expires_at: Some(1),
            ..live.clone()
        }),
        false,
    );
    assert!(served().await.is_none(), "an expired invite is refused");
    joiner(None, false);
    assert!(served().await.is_none(), "a revoked invite is refused");

    // The hint request (S7c.5) is admitted the same way.
    let hint = rekindle_codec::checkpoint_hint::CheckpointHintRequest {
        community: "c1".into(),
        height: 0,
        digest: [0; 32],
        requester: stranger.verifying_key().to_bytes(),
        invite: None,
    }
    .with_invite_code(&code)
    .encode(&stranger)
    .unwrap();
    let hinted = || async { host.governance().answer_checkpoint_hint(&hint).await };
    assert!(
        hinted().await.is_none(),
        "a hint for a revoked invite is refused"
    );
    joiner(Some(live), false);
    assert!(
        hinted().await.is_some(),
        "a joiner with a live invite is answered"
    );
    host.shutdown().await.unwrap();
}
