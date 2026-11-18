//! [`super::append_at_unless_logged`] over the in-memory port of `tests.rs`.

use rekindle_types::dht_layout::member_record;

use super::tests::{entries, MemPort};
use super::{append_at_unless_logged, read_log_after, LoggedAppend};
use crate::port::RecordPort;
use crate::schema;

async fn member_record() -> (MemPort, String, String) {
    let port = MemPort::default();
    let (lease, key, owner) = port.create(&schema::member_record(), None).await.unwrap();
    port.release(lease).await;
    (port, key, owner)
}

/// Plan E3.2 S7c.9: a fresh op log's first append reads its unwritten head
/// from the network once, for the check and the append both (each such
/// read is a fanout to its 10 s timeout on Veilid); a retry of an entry
/// already logged writes nothing.
#[tokio::test]
async fn a_first_append_reads_its_head_once_and_a_logged_entry_is_not_rewritten() {
    let (port, key, owner) = member_record().await;
    let gov_head = member_record::GOV_HEAD;
    let op = entries(7..=7).remove(0);
    let appended = append_at_unless_logged(&port, &key, gov_head, &owner, 0, op.clone(), |e| {
        e == op.as_slice()
    })
    .await
    .unwrap();
    assert!(matches!(appended, LoggedAppend::Appended(ref p) if p.first_seq == 1));
    assert_eq!(port.network_reads(), 1, "one network read of the head");

    let again = append_at_unless_logged(&port, &key, gov_head, &owner, 0, op.clone(), |e| {
        e == op.as_slice()
    })
    .await
    .unwrap();
    assert_eq!(again, LoggedAppend::AlreadyLogged);
    let read = read_log_after(&port, &key, gov_head, 0).await.unwrap();
    assert_eq!(
        read.entries.len(),
        1,
        "the logged entry is not appended twice"
    );
}
