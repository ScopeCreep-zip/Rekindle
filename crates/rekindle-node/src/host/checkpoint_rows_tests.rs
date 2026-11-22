//! The Host's checkpoint rows (plan E3.2 S7c.6), offline: a rebuild writes
//! each checkpoint digest's state, a usable hint's snapshot is kept on its
//! row once served, and it is not read again once the checkpoint is
//! equivocated.

use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::checkpoint::{
    checkpoint_digest, eligible_set, eligible_set_hash, frontier_hash, state_preimage, state_root,
};
use rekindle_governance::merge::merge_cut;
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_governance_runtime::checkpoint_state::snapshot_at;
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::op_log::{rebuild_ops, verify_held, HeldOp, OpFate};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{
    AdmissionMode, Checkpoint, GenesisCert, GovernanceEntry, GovernanceOp, OpRef,
};
use rekindle_types::id::PseudonymKey;

use super::{
    create_ann, meta, offline_host, seed_governance, seed_registry_community, signed_chain,
    CREATOR_RECORD,
};
use crate::host::Host;

fn locator(op: &HeldOp) -> OpRef {
    OpRef {
        op_hash: op.hash,
        device_key: op.device_key,
        author: op.author.clone(),
        author_record: CREATOR_RECORD.into(),
        author_seq: op.seq,
    }
}

/// `key`'s op at `seq` after `prev`, citing `deps`, signed as production
/// signs it.
fn signed(key: &SigningKey, prev: &HeldOp, deps: &[&HeldOp], body: GovernanceEntry) -> HeldOp {
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let seq = prev.seq + 1;
    let mut op = GovernanceOp {
        device_key: author.0,
        author: author.clone(),
        seq,
        prev: Some(prev.hash),
        deps: deps.iter().map(|d| locator(d)).collect(),
        issued_at: seq + 1,
        body,
        sig: [0; 64],
    };
    let unsigned = op_unsigned_bytes(&op);
    op.sig = rekindle_secrets::derive::sign_with_pseudonym(key, &op_signing_bytes("c1", &unsigned));
    HeldOp {
        hash: op_hash(&unsigned, &op.sig),
        device_key: author.0,
        author,
        author_record: CREATOR_RECORD.into(),
        seq,
        lamport: seq + 1,
        issued_at: seq + 1,
        encoded: encode_op(&op).unwrap(),
        fate: OpFate::Applied,
    }
}

/// The height-1 checkpoint over `first`'s op (the creator, the height-1
/// electorate alone) and the snapshot at its cut.
pub(super) fn height_one(cert: &GenesisCert, first: &[HeldOp]) -> (Checkpoint, Vec<u8>) {
    let (at_cut, cut) = merge_cut(cert, &verify_held("c1", first).ops);
    let preimage = state_preimage(&at_cut, &cut);
    let checkpoint = Checkpoint {
        height: 1,
        round: 0,
        prev: None,
        cut: vec![locator(&first[0])],
        frontier_hash: frontier_hash(&[first[0].hash]),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: state_root(&at_cut, &cut),
        state_size: u32::try_from(preimage.len()).expect("fits"),
        mek_generation: 0,
        eligible_set_hash: eligible_set_hash(&eligible_set(cert, None)),
        pages: Vec::new(),
    };
    (checkpoint, preimage)
}

/// Ann's `c1` whose creator endorsed a height-1 checkpoint over its first
/// op, followed by `later` more ops: the creator's key, its ops, the
/// checkpoint and the snapshot at its cut.
async fn community_with(
    host: &Host,
    later: usize,
) -> (SigningKey, Vec<HeldOp>, Checkpoint, Vec<u8>) {
    let ann = create_ann(host).await;
    seed_registry_community(host, &ann.public_key).await;
    let key = SigningKey::from_bytes(&[0xbb; 32]);
    let creator = PseudonymKey(key.verifying_key().to_bytes());
    let cert = rekindle_governance::fixture::cert(&creator, AdmissionMode::default());
    let first = signed_chain("c1", &key, CREATOR_RECORD, vec![meta("C")]);
    let (checkpoint, preimage) = height_one(&cert, &first);
    let mut bodies = vec![meta("C"), GovernanceEntry::Checkpoint(checkpoint.clone())];
    bodies.extend((0..later).map(|i| meta(&format!("C{i}"))));
    let ops = signed_chain("c1", &key, CREATOR_RECORD, bodies);
    seed_governance(host, &ann.public_key, "c1", &key, &ops).await;
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .genesis_cert = Some(cert);
    (key, ops, checkpoint, preimage)
}

/// Ann's `c1` with the height-1 checkpoint as its last op.
pub(super) async fn community(host: &Host) -> (SigningKey, Vec<HeldOp>, Checkpoint, Vec<u8>) {
    community_with(host, 0).await
}

/// Veilid's callee answer window (`veilid-core/src/rpc_processor/rpc_app_call.rs:131-138`,
/// `network.rpc.timeout_ms` 5,000 ms).
const APP_CALL_ANSWER: std::time::Duration = std::time::Duration::from_secs(5);

/// Plan E3.2 S7c.8: every chunk request after the first is served from the
/// kept row, not by loading and verifying the op set to find the
/// checkpoint. Measured debug at 10,000 ops: that search took 58-60 s per
/// chunk with every verdict cached (0 signature verifications: the
/// dependency-order walk of `verify_held_cached`), against a 5 s answer
/// window. At 3,000 ops it is several seconds; the kept row answers in a
/// small share of the window.
#[tokio::test]
async fn a_kept_snapshot_answers_each_chunk_without_the_op_set() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let (_, _, checkpoint, preimage) = community_with(&host, 3_000).await;
    let digest = checkpoint_digest(&checkpoint);
    let gov = host.governance();
    rebuild_ops(&gov, "c1").await.unwrap();
    assert_eq!(snapshot_at(&gov, "c1", 1, &digest).await.unwrap(), preimage);
    let started = std::time::Instant::now();
    for _ in 0..3 {
        assert_eq!(snapshot_at(&gov, "c1", 1, &digest).await.unwrap(), preimage);
    }
    let per_chunk = started.elapsed() / 3;
    assert!(
        per_chunk < APP_CALL_ANSWER / 10,
        "a kept chunk took {per_chunk:?}, over a tenth of the {APP_CALL_ANSWER:?} window"
    );
    host.shutdown().await.unwrap();
}

/// Plan E3.2 S7c.6: the rebuild marks the checkpoint usable, serving it
/// keeps its snapshot on its row, and a second majority digest for the
/// same cut (the creator's round-1 endorsement of another state) marks
/// both equivocated, so the kept snapshot is not read again.
#[tokio::test]
async fn a_kept_snapshot_whose_checkpoint_is_later_equivocated_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let (key, ops, checkpoint, preimage) = community(&host).await;
    let digest = checkpoint_digest(&checkpoint);
    let gov = host.governance();
    rebuild_ops(&gov, "c1").await.unwrap();
    assert_eq!(
        gov.usable_checkpoint_snapshot("c1", 1, digest)
            .await
            .unwrap(),
        None
    );
    let served = snapshot_at(&gov, "c1", 1, &digest).await.unwrap();
    assert_eq!(served, preimage);
    assert_eq!(
        gov.usable_checkpoint_snapshot("c1", 1, digest)
            .await
            .unwrap(),
        Some(preimage.clone()),
        "a usable hint's snapshot is kept once served"
    );
    assert_eq!(snapshot_at(&gov, "c1", 1, &digest).await.unwrap(), preimage);

    let other = Checkpoint {
        round: 1,
        state_root: [9; 32],
        ..checkpoint
    };
    let fork = signed(
        &key,
        &ops[1],
        &[&ops[0], &ops[1]],
        GovernanceEntry::Checkpoint(other),
    );
    gov.store_governance_op("c1", &fork).await.unwrap();
    rebuild_ops(&gov, "c1").await.unwrap();
    assert_eq!(
        gov.usable_checkpoint_snapshot("c1", 1, digest)
            .await
            .unwrap(),
        None,
        "an equivocated checkpoint's kept snapshot is not read"
    );
    host.shutdown().await.unwrap();
}

/// IJ3: a community we no longer hold (a join withdrawn, SU1's local-only
/// forget) stores no op a late ring change or walk read before the forget:
/// the store is refused as `CommunityNotFound`, never written.
#[tokio::test]
async fn an_op_for_a_community_no_longer_held_is_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let (key, ops, _, _) = community(&host).await;
    let gov = host.governance();
    let held = gov.governance_ops("c1").await.unwrap().len();
    host.ctx.communities.map().write().remove("c1");
    let late = signed(&key, ops.last().unwrap(), &[], meta("late"));
    let stored = gov.store_governance_op("c1", &late).await;
    assert!(
        matches!(
            stored,
            Err(rekindle_governance_runtime::GovernanceRuntimeError::CommunityNotFound(_))
        ),
        "{stored:?}"
    );
    assert_eq!(gov.governance_ops("c1").await.unwrap().len(), held);
    host.shutdown().await.unwrap();
}
