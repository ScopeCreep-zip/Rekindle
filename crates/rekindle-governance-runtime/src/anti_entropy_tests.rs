//! A frontier whose heads did not all fetch stays unread: the next
//! inspect reads it again although its seq has not moved. A frontier with
//! our heads and another roster checksum rebuilds our state (D15).

use super::*;
use crate::deps::MockGovernanceRuntimeDeps;
use rekindle_records::lease::LeaseId;
use rekindle_types::governance::{Frontier, OpRef, RosterChecksum};
use rekindle_types::id::PseudonymKey;

fn frontier() -> Vec<u8> {
    rekindle_codec::governance_op::encode_frontier(&Frontier {
        heads: vec![OpRef {
            op_hash: OpHash([7; 32]),
            author: PseudonymKey([1; 32]),
            author_record: "VLD0:m".into(),
            author_seq: 0,
        }],
        roster_checksum: RosterChecksum([0; 32]),
    })
    .expect("frontier encodes")
}

/// `deps` for a record at frontier seq 5 whose one head never fetches,
/// read `reads` times.
fn deps(reads: usize) -> MockGovernanceRuntimeDeps {
    let mut deps = MockGovernanceRuntimeDeps::new();
    deps.expect_acquire_record()
        .returning(|_, _, _| Ok(LeaseId(1)));
    deps.expect_release_record().returning(|_| ());
    deps.expect_inspect_dht_record_update_get_seqs()
        .returning(|_| {
            let mut seqs = vec![None; member_record::SUBKEY_COUNT as usize];
            seqs[member_record::FRONTIER as usize] = Some(5);
            Ok(seqs)
        });
    deps.expect_get_dht_value()
        .times(reads)
        .returning(|_, _, _| Ok(Some(frontier())));
    deps.expect_fetch_governance_op().returning(|_| Ok(None));
    deps
}

#[tokio::test]
async fn a_head_that_did_not_fetch_leaves_the_frontier_unread() {
    let deps = deps(2);
    let mut seen = SeenFrontiers::new();
    let mut known = BTreeSet::new();
    for _ in 0..2 {
        let stored = inspect_one(&deps, "A", "VLD0:m", &mut seen, &mut known)
            .await
            .expect("inspected");
        assert_eq!(stored, 0);
    }
    assert!(seen.is_empty(), "{seen:?}");
    assert!(known.is_empty(), "an unfetched head is not known");
}

/// An op whose encoding hashes to `OpRef.op_hash` (`held_from_encoded`
/// checks the hash, not the signature) and the frontier naming it.
fn fetchable() -> (Vec<u8>, Vec<u8>) {
    let op = rekindle_types::governance::GovernanceOp {
        author: PseudonymKey([1; 32]),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body: rekindle_types::governance::GovernanceEntry::CommunityMeta {
            name: Some("a".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
        sig: [0; 64],
    };
    let unsigned = rekindle_codec::governance_op::op_unsigned_bytes(&op);
    let hash = rekindle_governance::op::op_hash(&unsigned, &op.sig);
    let encoded = rekindle_codec::governance_op::encode_op(&op).expect("op encodes");
    let frontier = rekindle_codec::governance_op::encode_frontier(&Frontier {
        heads: vec![OpRef {
            op_hash: hash,
            author: PseudonymKey([1; 32]),
            author_record: "VLD0:m".into(),
            author_seq: 0,
        }],
        roster_checksum: RosterChecksum([0; 32]),
    })
    .expect("frontier encodes");
    (encoded, frontier)
}

/// A head that did not fetch on the first read is fetched on the next read
/// at the same seq and stored; only then is the seq read, so a third
/// inspect does not read the frontier again. Before d1f6493b the first
/// read marked the head known and the seq read, so the op was never
/// stored.
#[tokio::test]
async fn a_head_that_fetches_on_a_later_read_is_stored_and_then_the_seq_is_read() {
    let (encoded, frontier) = fetchable();
    let mut deps = MockGovernanceRuntimeDeps::new();
    deps.expect_acquire_record()
        .returning(|_, _, _| Ok(LeaseId(1)));
    deps.expect_release_record().returning(|_| ());
    deps.expect_inspect_dht_record_update_get_seqs()
        .returning(|_| {
            let mut seqs = vec![None; member_record::SUBKEY_COUNT as usize];
            seqs[member_record::FRONTIER as usize] = Some(5);
            Ok(seqs)
        });
    deps.expect_get_dht_value()
        .times(2)
        .returning(move |_, _, _| Ok(Some(frontier.clone())));
    let fetches = std::sync::atomic::AtomicUsize::new(0);
    deps.expect_fetch_governance_op().returning(move |_| {
        let first = fetches.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        Ok((!first).then(|| encoded.clone()))
    });
    deps.expect_store_governance_op()
        .times(1)
        .returning(|_, _| Ok(()));
    let mut seen = SeenFrontiers::new();
    let mut known = BTreeSet::new();
    let mut stored = Vec::new();
    for _ in 0..3 {
        stored.push(
            inspect_one(&deps, "A", "VLD0:m", &mut seen, &mut known)
                .await
                .expect("inspected"),
        );
    }
    assert_eq!(stored, [0, 1, 0]);
    assert_eq!(seen.get("VLD0:m"), Some(&5));
}

/// `deps` holding one signed op of ours, whose record's frontier names
/// exactly that op as its head with `checksum`; the rebuild entry point
/// ([`crate::rebuild_governance`]) is reached `rebuilds` times, seen
/// through its first call (the joined-community check, which ends it here).
/// Returns `deps` and the op set's hashes, our `known`.
fn converging(
    checksum: RosterChecksum,
    rebuilds: usize,
) -> (MockGovernanceRuntimeDeps, BTreeSet<OpHash>) {
    use rekindle_secrets::ed25519_dalek::SigningKey;
    let key = SigningKey::from_bytes(&[9; 32]);
    let cert = rekindle_types::governance::GenesisCert {
        name: "c".into(),
        policy: rekindle_types::governance::AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey(key.verifying_key().to_bytes()),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
    };
    let state = rekindle_governance::state::GovernanceState::default();
    let (op, encoded) = op_log::build_op(
        "A",
        &cert,
        &state,
        &op_log::VerifiedSet::default(),
        &key,
        rekindle_governance::fixture::meta("a"),
        5,
    )
    .expect("op builds");
    let held = op_log::held(&op, "VLD0:m".into(), encoded, op_log::OpFate::Applied);
    let known = BTreeSet::from([op.hash()]);
    let bytes = rekindle_codec::governance_op::encode_frontier(&Frontier {
        heads: vec![op.op_ref("VLD0:m")],
        roster_checksum: checksum,
    })
    .expect("frontier encodes");
    let mut deps = MockGovernanceRuntimeDeps::new();
    deps.expect_acquire_record()
        .returning(|_, _, _| Ok(LeaseId(1)));
    deps.expect_release_record().returning(|_| ());
    deps.expect_inspect_dht_record_update_get_seqs()
        .returning(|_| {
            let mut seqs = vec![None; member_record::SUBKEY_COUNT as usize];
            seqs[member_record::FRONTIER as usize] = Some(5);
            Ok(seqs)
        });
    deps.expect_get_dht_value()
        .times(1)
        .returning(move |_, _, _| Ok(Some(bytes.clone())));
    deps.expect_genesis_cert()
        .returning(move |_| Some(cert.clone()));
    deps.expect_governance_state()
        .returning(move |_| Some(state.clone()));
    deps.expect_governance_ops()
        .returning(move |_| Ok(vec![held.clone()]));
    deps.expect_list_community_governance_targets()
        .times(rebuilds)
        .returning(Vec::new);
    (deps, known)
}

/// Equal heads with another roster checksum: our merge disagrees with a
/// peer holding the same op set, so the state is rebuilt, once, and the
/// seq is read.
#[tokio::test]
async fn equal_heads_with_another_checksum_rebuild_the_state() {
    let (deps, mut known) = converging(RosterChecksum([0xEE; 32]), 1);
    let mut seen = SeenFrontiers::new();
    let stored = inspect_one(&deps, "A", "VLD0:m", &mut seen, &mut known)
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
    assert_eq!(seen.get("VLD0:m"), Some(&5));
}

/// Equal heads with our own checksum: nothing to rebuild.
#[tokio::test]
async fn equal_heads_with_our_checksum_do_not_rebuild() {
    let ours = roster_checksum(&rekindle_governance::state::GovernanceState::default());
    let (deps, mut known) = converging(ours, 0);
    let mut seen = SeenFrontiers::new();
    let stored = inspect_one(&deps, "A", "VLD0:m", &mut seen, &mut known)
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
    assert_eq!(seen.get("VLD0:m"), Some(&5));
}
