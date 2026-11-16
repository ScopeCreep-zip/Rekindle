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

#[tokio::test]
async fn a_read_frontier_at_an_unmoved_seq_is_not_read_again() {
    let deps = deps(0);
    let mut seen = SeenFrontiers::from([("VLD0:m".to_owned(), 5)]);
    let stored = inspect_one(&deps, "A", "VLD0:m", &mut seen, &mut BTreeSet::new())
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
}
