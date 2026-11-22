//! A community joined from a checkpoint hint is partial until its backfill
//! ends (plan E3.2 S7c.8 (1), Synapse `is_partial_state_room`), offline:
//! while partial it answers no hint, snapshot or bootstrap request and
//! makes no checkpoint proposal; the rebuild that holds every op ends it
//! and compares the cut's state with the hinted snapshot's digest.

use rekindle_codec::checkpoint_hint::CheckpointHintRequest;
use rekindle_codec::checkpoint_state::CheckpointStateRequest;
use rekindle_governance::checkpoint::checkpoint_digest;
use rekindle_governance_runtime::checkpoint_hint::answer_hint_request;
use rekindle_governance_runtime::checkpoint_state::answer_state_request;
use rekindle_governance_runtime::checkpointing::{tick, CheckpointClock, CheckpointStep};
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::op_log::{rebuild_ops, verify_held};
use rekindle_governance_runtime::partial::{finish_backfill, PartialJoin};
use rekindle_governance_runtime::{GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_secrets::derive::derive_community_pseudonym;
use rekindle_types::governance::{Checkpoint, OpRef};
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{GovernanceEvent, StreamFrame, SubscriptionEvent};

use super::checkpoint_rows::{community, height_one};
use super::{create_ann, meta, offline_host, seed_governance, seed_registry_community};
use super::{signed_chain, CREATOR_RECORD};

fn partial_from(checkpoint: &Checkpoint, state_root: [u8; 32]) -> PartialJoin {
    PartialJoin {
        height: checkpoint.height,
        digest: checkpoint_digest(checkpoint),
        state_root,
    }
}

fn is_partial_refusal<T>(result: &Result<T, GovernanceRuntimeError>) -> bool {
    matches!(result, Err(GovernanceRuntimeError::PartialState(c)) if c == "c1")
}

/// The false-snapshot events published so far.
async fn mismatches(stream: &mut rekindle_events::EventStream) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await
    {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::Governance(GovernanceEvent::CheckpointSnapshotMismatch {
                height,
                digest,
                ..
            }) = s.event
            {
                out.push((height, digest));
            }
        }
    }
    out
}

#[tokio::test]
async fn a_partial_community_answers_no_hint_or_snapshot_request() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let (_, _, checkpoint, _) = community(&host).await;
    let gov = host.governance();
    rebuild_ops(&gov, "c1").await.unwrap();
    let digest = checkpoint_digest(&checkpoint);
    let state = CheckpointStateRequest {
        community: "c1".into(),
        height: 1,
        digest,
        offset: 0,
        requester: [3; 32],
        invite: None,
    };
    let hint = CheckpointHintRequest {
        community: "c1".into(),
        height: 1,
        digest,
        requester: [3; 32],
        invite: None,
    };
    assert!(
        answer_state_request(&gov, &state).await.is_ok(),
        "whole: served"
    );
    assert!(
        answer_hint_request(&gov, &hint).await.is_ok(),
        "whole: answered"
    );

    gov.set_governance_partial("c1", Some(partial_from(&checkpoint, checkpoint.state_root)))
        .await
        .unwrap();
    assert!(is_partial_refusal(
        &answer_state_request(&gov, &state).await
    ));
    assert!(is_partial_refusal(&answer_hint_request(&gov, &hint).await));
    host.shutdown().await.unwrap();
}

/// Plan E3.2 S7c.8 (i): while partial, a bootstrap request is answered
/// with the typed `bootstrapPartial` (Synapse's
/// `UNABLE_DUE_TO_PARTIAL_STATE`), never a frontier and never a refusal;
/// whole, the same request is checked as usual.
#[tokio::test]
async fn a_partial_community_answers_bootstrap_partial() {
    use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
    use rekindle_governance_runtime::BootstrapAnswer;
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let (_, ops, checkpoint, _) = community(&host).await;
    let gov = host.governance();
    rebuild_ops(&gov, "c1").await.unwrap();
    let joiner = rekindle_governance_runtime::BootstrapJoiner {
        pseudonym: "00".repeat(32),
        member_joined: OpRef {
            op_hash: ops[0].hash,
            device_key: ops[0].device_key,
            author: ops[0].author.clone(),
            author_record: CREATOR_RECORD.into(),
            author_seq: 0,
        },
        member_joined_op: ops[0].encoded.clone(),
        route: Vec::new(),
    };
    let whole = rekindle_governance_runtime::answer_bootstrap_request(&gov, "c1", &joiner).await;
    assert!(whole.is_err(), "whole: the request is checked, {whole:?}");

    gov.set_governance_partial("c1", Some(partial_from(&checkpoint, checkpoint.state_root)))
        .await
        .unwrap();
    let answer = rekindle_governance_runtime::answer_bootstrap_request(&gov, "c1", &joiner).await;
    let Ok(BootstrapAnswer::Partial(bytes)) = answer else {
        panic!("partial: the typed answer, {answer:?}");
    };
    let decoded = rekindle_codec::capnp_envelope::try_decode_community_envelope(&bytes);
    assert!(
        matches!(
            decoded,
            Ok(Some(CommunityEnvelope::Control(ControlPayload::BootstrapPartial { ref anchor })))
                if anchor == "c1"
        ),
        "{decoded:?}"
    );
    host.shutdown().await.unwrap();
}

/// Our own community (its creator is our pseudonym in it, so the height-1
/// electorate) with one power op: a proposal is due once a sweep passes
/// with no newer op, unless the community is partial.
#[tokio::test]
async fn a_partial_community_proposes_no_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let gov = host.governance();
    let me = derive_community_pseudonym(&gov.identity_secret().unwrap(), "c1");
    let ops = signed_chain("c1", &me, CREATOR_RECORD, vec![meta("C")]);
    let cert = seed_governance(&host, &ann.public_key, "c1", &me, &ops).await;
    assert_eq!(
        cert.creator_pseudonym,
        PseudonymKey(me.verifying_key().to_bytes())
    );
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .genesis_cert = Some(cert);

    gov.set_governance_partial(
        "c1",
        Some(PartialJoin {
            height: 1,
            digest: [1; 32],
            state_root: [2; 32],
        }),
    )
    .await
    .unwrap();
    let mut clock = CheckpointClock::default();
    for _ in 0..2 {
        assert_eq!(
            tick(&gov, "c1", &mut clock).await.unwrap(),
            CheckpointStep::Idle
        );
    }

    gov.set_governance_partial("c1", None).await.unwrap();
    let mut clock = CheckpointClock::default();
    let _ = tick(&gov, "c1", &mut clock).await;
    let due = tick(&gov, "c1", &mut clock).await;
    assert!(
        !matches!(due, Ok(CheckpointStep::Idle)),
        "whole: due, {due:?}"
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_rebuild_that_holds_every_op_ends_a_partial_join() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let (_, _, checkpoint, _) = community(&host).await;
    let gov = host.governance();
    let mut events = host.ctx.journal.subscribe(None);
    gov.set_governance_partial("c1", Some(partial_from(&checkpoint, checkpoint.state_root)))
        .await
        .unwrap();
    rebuild_ops(&gov, "c1").await.unwrap();
    assert_eq!(gov.governance_partial("c1").await.unwrap(), None, "whole");
    assert!(
        mismatches(&mut events).await.is_empty(),
        "the snapshot was true"
    );
    host.shutdown().await.unwrap();
}

/// Threat T8: a strictly endorsed snapshot that is not the op set's state at
/// its cut is reported once the backfill shows it, and the community stays
/// on the backfilled state.
#[tokio::test]
async fn a_false_hinted_snapshot_is_reported_when_the_backfill_ends() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let (_, _, checkpoint, _) = community(&host).await;
    let gov = host.governance();
    let mut events = host.ctx.journal.subscribe(None);
    gov.set_governance_partial("c1", Some(partial_from(&checkpoint, [0xEE; 32])))
        .await
        .unwrap();
    let state = rebuild_ops(&gov, "c1").await.unwrap();
    assert_eq!(gov.governance_partial("c1").await.unwrap(), None, "whole");
    assert_eq!(
        mismatches(&mut events).await,
        [(1, hex::encode(checkpoint_digest(&checkpoint)))]
    );
    assert_eq!(state.metadata.as_ref().map(|m| m.name.as_str()), Some("C"));
    host.shutdown().await.unwrap();
}

/// A restart mid-backfill: the flag is read back from the store, and a
/// rebuild that cannot fetch the cut leaves the community partial.
#[tokio::test]
async fn a_rebuild_still_missing_the_cut_stays_partial() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let key = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[0xbb; 32]);
    let ops = signed_chain("c1", &key, CREATOR_RECORD, vec![meta("C"), meta("D")]);
    let cert = seed_governance(&host, &ann.public_key, "c1", &key, &ops[1..]).await;
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .genesis_cert = Some(cert);
    let gov = host.governance();
    let hint = PartialJoin {
        height: 1,
        digest: [1; 32],
        state_root: [2; 32],
    };
    gov.set_governance_partial("c1", Some(hint)).await.unwrap();
    let _ = rebuild_ops(&gov, "c1").await;
    assert_eq!(gov.governance_partial("c1").await.unwrap(), Some(hint));
    host.shutdown().await.unwrap();
}

/// S7c.8 (iii): the hint's checkpoint op is held and verified, and the op
/// set still misses an op (the creator's op after it, cited by the next
/// one held, which waits on it): the backfill has not ended, so the
/// community stays partial although the held cut reproduces the hinted
/// snapshot. `finish_backfill` is driven with the verified set a rebuild
/// hands it once a pass fetches nothing new; an offline Host's
/// `rebuild_ops` stops at its first fetch (no transport) and never reaches
/// it, which is why `a_rebuild_still_missing_the_cut_stays_partial` holds
/// without the check.
#[tokio::test]
async fn a_set_holding_the_checkpoint_but_missing_an_op_stays_partial() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let key = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[0xbb; 32]);
    let creator = PseudonymKey(key.verifying_key().to_bytes());
    let cert = rekindle_governance::fixture::cert(
        &creator,
        rekindle_types::governance::AdmissionMode::default(),
    );
    let first = signed_chain("c1", &key, CREATOR_RECORD, vec![meta("C")]);
    let (checkpoint, _) = height_one(&cert, &first);
    let ops = signed_chain(
        "c1",
        &key,
        CREATOR_RECORD,
        vec![
            meta("C"),
            rekindle_types::governance::GovernanceEntry::Checkpoint(checkpoint.clone()),
            meta("D"),
            meta("E"),
        ],
    );
    let set = verify_held("c1", &[ops[0].clone(), ops[1].clone(), ops[3].clone()]);
    assert_eq!(set.ops.len(), 2, "the checkpoint op is verified");
    assert_eq!(set.missing.len(), 1, "the op after it is missing");
    let gov = host.governance();
    let hint = partial_from(&checkpoint, checkpoint.state_root);
    gov.set_governance_partial("c1", Some(hint)).await.unwrap();
    finish_backfill(&gov, "c1", &cert, &set).await.unwrap();
    assert_eq!(gov.governance_partial("c1").await.unwrap(), Some(hint));
    // Held whole, the same checkpoint ends the backfill.
    finish_backfill(&gov, "c1", &cert, &verify_held("c1", &ops))
        .await
        .unwrap();
    assert_eq!(gov.governance_partial("c1").await.unwrap(), None);
    host.shutdown().await.unwrap();
}

/// Plan E3.2 S7c.8 (2): pre-join reads are served for a live invite only,
/// to any number of askers (each proved the code, `admit_reader`); a
/// revoked, expired or used-up invite admits nobody.
#[tokio::test]
async fn pre_join_reads_need_a_live_invite() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let invite = rekindle_governance::state::InviteState {
        code_hash: "code".into(),
        max_uses: 2,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
        created: rekindle_governance::op::OpStamp::MIN,
        creator_pseudonym: PseudonymKey([2; 32]),
        uses: 0,
    };
    let set_invite = |invite: Option<rekindle_governance::state::InviteState>| {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        let state = c.governance_state.get_or_insert_with(Default::default);
        state.invites.clear();
        state.invites.extend(invite.map(|i| ([1; 16], i)));
    };
    set_invite(Some(invite.clone()));
    let gov = host.governance();
    let reader = |n: u8| PseudonymKey([n; 32]);
    for n in 10..14 {
        assert!(gov.holds_live_invite("c1", &reader(n), "code"), "{n}");
    }
    assert!(
        !gov.holds_live_invite("c1", &reader(10), "other"),
        "unknown"
    );

    set_invite(None);
    assert!(!gov.holds_live_invite("c1", &reader(10), "code"), "revoked");
    set_invite(Some(rekindle_governance::state::InviteState {
        expires_at: Some(1),
        ..invite.clone()
    }));
    assert!(!gov.holds_live_invite("c1", &reader(10), "code"), "expired");
    set_invite(Some(rekindle_governance::state::InviteState {
        uses: 2,
        ..invite
    }));
    assert!(!gov.holds_live_invite("c1", &reader(10), "code"), "used up");
    host.shutdown().await.unwrap();
}
