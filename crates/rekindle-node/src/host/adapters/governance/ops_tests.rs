//! The op log append is idempotent across a row's attempts.

use rekindle_governance::fixture;
use rekindle_types::dht_layout::member_record::GOV_HEAD;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use super::append_op_once;

#[tokio::test]
async fn two_attempts_of_one_row_leave_one_log_entry() {
    let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 82);
    let node = nodes.remove(0);
    let _pump = sim.start();
    sim.quiesce().await;
    let port = node.ports.records.start_session().unwrap();
    let (lease, record, owner) = port
        .create(&rekindle_records::schema::member_record(), None)
        .await
        .unwrap();
    port.release(lease).await;
    let author = PseudonymKey([3; 32]);
    let op = fixture::op(
        &author,
        0,
        &[],
        1,
        GovernanceEntry::CommunityMeta {
            name: Some("Ops".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
    )
    .op()
    .clone();
    let encoded = rekindle_codec::governance_op::encode_op(&op).unwrap();
    // The first attempt lands; its acknowledgement is lost and the row is
    // attempted again.
    assert!(append_op_once(&*port, &record, &owner, encoded.clone())
        .await
        .unwrap()
        .is_some());
    assert!(append_op_once(&*port, &record, &owner, encoded.clone())
        .await
        .unwrap()
        .is_none());
    let log = rekindle_records::chain::read_log_after(&*port, &record, GOV_HEAD, 0)
        .await
        .unwrap();
    assert_eq!(log.last_seq, 1);
    assert_eq!(log.entries.len(), 1);
    assert_eq!(log.entries[0].ciphertext, encoded);
}

/// A head write that only two replicas took (`Fault::WritePartial`) is
/// readable but below consensus; the row's retry finds its op in the log and
/// republishes the head until `set_value_count` replicas hold it.
#[tokio::test(start_paused = true)]
async fn a_retry_that_finds_its_op_lands_a_partial_head() {
    let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 83);
    let node = nodes.remove(0);
    let _pump = sim.start();
    sim.quiesce().await;
    let port = node.ports.records.start_session().unwrap();
    let (lease, record, owner) = port
        .create(&rekindle_records::schema::member_record(), None)
        .await
        .unwrap();
    port.release(lease).await;
    let author = PseudonymKey([4; 32]);
    let op = fixture::op(
        &author,
        0,
        &[],
        1,
        GovernanceEntry::CommunityMeta {
            name: Some("Partial".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
    )
    .op()
    .clone();
    let encoded = rekindle_codec::governance_op::encode_op(&op).unwrap();
    sim.fault(rekindle_netsim::Fault::WritePartial {
        node: 0,
        of: rekindle_netsim::Writes::Record(record.clone()),
        accepting: 2,
    });
    assert!(append_op_once(&*port, &record, &owner, encoded.clone())
        .await
        .is_err());
    sim.fault(rekindle_netsim::Fault::Heal);
    let holders = || {
        sim.replica_values(&record, GOV_HEAD)
            .into_iter()
            .flatten()
            .count()
    };
    assert_eq!(holders(), 2, "the partial head");
    assert!(append_op_once(&*port, &record, &owner, encoded.clone())
        .await
        .unwrap()
        .is_none());
    assert!(holders() >= 5, "the head landed: {} holders", holders());
    let log = rekindle_records::chain::read_log_after(&*port, &record, GOV_HEAD, 0)
        .await
        .unwrap();
    assert_eq!(log.entries.len(), 1);
}
