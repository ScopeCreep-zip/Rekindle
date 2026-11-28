//! Open intent through the pool (plan VU3, decision 1): a one-shot read,
//! inspect or write and an existence probe open as `Check`, which never
//! asks Veilid to rehydrate; our own records, the keepalive and long-lived
//! leases open as `Hold`, which does when it is due (0.5.8 `open_record.rs:62-81`,
//! modelled and tested in the netsim's `rehydrate_tests`). A hinted read
//! goes to the network for every subkey a value change named.

use std::sync::Arc;

use rekindle_records::port::{OpenIntent, RecordPort, ReportScope};

use crate::test_support::{pool_on, MemDht};

const PEER_RECORD: &str = "peer-record";

#[tokio::test]
async fn one_shot_reads_inspects_and_writes_open_as_check() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    pool.read_once(PEER_RECORD, 0, true).await.unwrap();
    pool.inspect_once(PEER_RECORD, None, ReportScope::UpdateGet)
        .await
        .unwrap();
    pool.write_once(PEER_RECORD, 0, b"v".to_vec(), Some("w"))
        .await
        .unwrap();
    assert_eq!(
        dht.opens(),
        vec![(PEER_RECORD.to_string(), OpenIntent::Check); 3],
        "every one-shot borrow is a check"
    );
}

#[tokio::test]
async fn a_held_record_keeps_its_open_and_a_writer_upgrade_holds() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let checked = RecordPort::acquire(&*pool, PEER_RECORD, None, OpenIntent::Check)
        .await
        .unwrap();
    // A second borrow is a table hit: no open, the first open's choice.
    let hit = RecordPort::acquire(&*pool, PEER_RECORD, None, OpenIntent::Hold)
        .await
        .unwrap();
    assert_eq!(dht.opens().len(), 1);
    // A writer is custody: the upgrade re-opens as Hold whatever was asked.
    let writable = RecordPort::acquire(&*pool, PEER_RECORD, Some("w"), OpenIntent::Check)
        .await
        .unwrap();
    assert_eq!(
        dht.opens(),
        vec![
            (PEER_RECORD.to_string(), OpenIntent::Check),
            (PEER_RECORD.to_string(), OpenIntent::Hold)
        ]
    );
    for lease in [checked, hit, writable] {
        pool.release(lease).await;
    }
}

#[tokio::test]
async fn the_keepalive_rehydrate_reopens_as_hold() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let lease = RecordPort::acquire(&*pool, PEER_RECORD, None, OpenIntent::Check)
        .await
        .unwrap();
    pool.rehydrate(lease).await.unwrap();
    assert_eq!(
        dht.opens().last(),
        Some(&(PEER_RECORD.to_string(), OpenIntent::Hold)),
        "a rehydration asks for one even on a record first opened as a check"
    );
    pool.release(lease).await;
}

#[tokio::test]
async fn a_hinted_read_asks_the_network_for_every_hinted_subkey() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let lease = RecordPort::acquire(&*pool, PEER_RECORD, Some("w"), OpenIntent::Hold)
        .await
        .unwrap();
    for subkey in [2, 4] {
        pool.set(lease, subkey, vec![u8::try_from(subkey).unwrap()], None)
            .await
            .unwrap();
    }
    let before = dht.gets().len();
    let values = pool.read_hinted(lease, &[4, 2, 4]).await.unwrap();
    assert_eq!(
        values
            .iter()
            .map(|(s, v)| (*s, v.data.clone()))
            .collect::<Vec<_>>(),
        vec![(2, vec![2]), (4, vec![4])]
    );
    let mut gets = dht.gets().split_off(before);
    gets.sort();
    assert_eq!(
        gets,
        vec![
            (PEER_RECORD.to_string(), 2, true),
            (PEER_RECORD.to_string(), 4, true)
        ],
        "each hinted subkey once, from the network, whatever an inspect would say"
    );
    pool.release(lease).await;
}
