//! A frontier whose heads did not all fetch stays unread: the next
//! inspect reads it again although its seq has not moved.

use super::*;
use crate::deps::MockGovernanceRuntimeDeps;
use rekindle_records::lease::LeaseId;
use rekindle_types::governance::OpRef;
use rekindle_types::id::PseudonymKey;

fn frontier() -> Vec<u8> {
    rekindle_codec::governance_op::encode_frontier(&[OpRef {
        op_hash: OpHash([7; 32]),
        author: PseudonymKey([1; 32]),
        author_record: "VLD0:m".into(),
        author_seq: 0,
    }])
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
    let frontier = rekindle_codec::governance_op::encode_frontier(&[OpRef {
        op_hash: hash,
        author: PseudonymKey([1; 32]),
        author_record: "VLD0:m".into(),
        author_seq: 0,
    }])
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
