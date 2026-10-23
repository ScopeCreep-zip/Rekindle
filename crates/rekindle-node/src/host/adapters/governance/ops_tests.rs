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
        .unwrap());
    assert!(!append_op_once(&*port, &record, &owner, encoded.clone())
        .await
        .unwrap());
    let log = rekindle_records::chain::read_log_after(&*port, &record, GOV_HEAD, 0)
        .await
        .unwrap();
    assert_eq!(log.last_seq, 1);
    assert_eq!(log.entries.len(), 1);
    assert_eq!(log.entries[0].ciphertext, encoded);
}
